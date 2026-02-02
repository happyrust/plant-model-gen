use crate::consts::*;
use crate::data_interface::db_meta_manager::db_meta;
use crate::data_interface::db_model::TUBI_TOL;
use crate::data_interface::interface::PdmsDataInterface;
use crate::fast_model;
use crate::fast_model::gen_model::cate_helpers::cal_sjus_value;
use crate::fast_model::gen_model::cate_single::{CateCsgShapeMap, gen_cata_single_geoms};
use crate::fast_model::gen_model::utilities::is_valid_cata_hash;
use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::refno_errors::{RefnoErrorKind, RefnoErrorStage, record_refno_error};
use crate::fast_model::{SEND_INST_SIZE, get_generic_type, resolve_desi_comp, shared};
use crate::fast_model::{debug_model, debug_model_debug};
use aios_core::consts::{CIVIL_TYPES, NGMR_OWN_TYPES};
use aios_core::geometry::*;
use aios_core::options::DbOption;
use crate::options::DbOptionExt;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::parsed_data::{CateAxisParam, CateGeomsInfo, TubiInfoData};
use aios_core::pdms_types::*;
use aios_core::pe::SPdmsElement;
use aios_core::prim_geo::basic::{BOXI_GEO_HASH, TUBI_GEO_HASH};
use aios_core::prim_geo::category::{CateCsgShape, try_convert_cate_geo_to_csg_shape};
use aios_core::prim_geo::profile::create_profile_geos;
use aios_core::prim_geo::*;
use aios_core::prim_geo::{PdmsTubing, TubiEdge};
use aios_core::shape::pdms_shape::{BrepShapeTrait, PlantMesh, RsVec3, VerifiedShape};
use aios_core::tool::math_tool::to_pdms_vec_str;
use aios_core::{
    HASH_PSEUDO_ATT_MAPS, NamedAttrMap, NamedAttrValue, RefU64, RefnoEnum, SUL_DB, gen_aabb_hash, gen_bevy_transform_hash,
};
use bevy_transform::components::Transform;
use dashmap::DashMap;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use glam::{DMat4, DVec3, Quat, Vec3};
use nalgebra::Point3;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use parry3d::bounding_volume::*;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::mem::take;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};

// #[cfg(feature = "profile")]
use tracing::{Level, info_span, instrument};

// Chrome tracing（统一走 crate::profiling，避免重复 init）
#[cfg(feature = "profile")]
pub fn init_chrome_tracing() -> anyhow::Result<()> {
    // 旧逻辑固定写到根目录；现在主要用于“兜底”。
    // 正常情况下应由 gen_model/orchestrator 在更上层先初始化（带 dbnum/时间戳）。
    crate::profiling::init_chrome_tracing("output/profile/chrome_trace_cata_model.json")
}

#[derive(Debug, Default, IntoPrimitive, Eq, PartialEq, TryFromPrimitive, Copy, Clone)]
#[repr(i32)]
pub enum NgmrRemovedType {
    #[default]
    AsDefault = -1,
    Nothing = 0,
    Attached = 1,
    Owner = 2,
    Item = 3,
    AttachedAndOwner = 4,
    AttachedAndItem = 5,
    OwnerAndItem = 6,
    All = 7,
}

/// 普通 CATE 生成阶段的输出
pub struct CateGenOutcome {
    pub local_al_map: Arc<DashMap<RefnoEnum, [CateAxisParam; 2]>>,
    /// tubi_info 收集器: 组合键 ID -> TubiInfoData
    pub tubi_info_map: Arc<DashMap<String, TubiInfoData>>,
    pub time_stats: HashMap<String, u64>,
    pub unique_cata_cnt: usize,
    pub elapsed_ms: u128,
}

fn build_tubi_transform_from_segment(
    start: Vec3,
    end: Vec3,
    tubi_size: &TubiSize,
) -> Option<Transform> {
    let dir = end - start;
    let dist = dir.length();
    if dist <= TUBI_TOL {
        return None;
    }
    let axis = dir / dist;
    let rot = if axis.length_squared() > 0.0 {
        Quat::from_rotation_arc(Vec3::Z, axis)
    } else {
        Quat::IDENTITY
    };
    let (sx, sy) = match tubi_size {
        TubiSize::BoreSize(b) => (*b, *b),
        TubiSize::BoxSize((h, w)) => (*w, *h),
        _ => (1.0, 1.0),
    };
    let mut t = Transform::IDENTITY;
    t.translation = start;
    t.rotation = rot;
    t.scale = Vec3::new(sx, sy, dist);
    Some(t)
}

/// BRAN/HANG tubing 生成阶段的输出
pub struct BranchTubiOutcome {
    pub tubi_relates: Vec<String>,
    pub tubi_refnos: Vec<String>,
    pub time_stats: HashMap<String, u64>,
    pub tubi_count: i32,
    pub elapsed_ms: u128,
}

pub struct GenOutcome {
    pub cate: Option<CateGenOutcome>,
    pub branch: Option<BranchTubiOutcome>,
}

// gen_cata_single_geoms 已移至 gen_model/cate_single.rs
// cal_sjus_value 已移至 gen_model/cate_helpers.rs

/// 生成元件库的branch型几何体
/// 动态修改tubi，还是要单独出来, 还是直接去修改整个bran？
/// 先暂时整个重新生成？
pub async fn gen_cata_geos(
    db_option: Arc<DbOptionExt>,
    target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
    branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>,
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<bool> {
    gen_cata_geos_inner(
        db_option,
        target_cata_map,
        branch_map,
        sjus_map_arc,
        sender,
        Arc::new(DashMap::new()),
        true,
        true,
    )
    .await
    .map(|_| true)
}

/// 仅处理普通 CATE 元件库几何体（不处理 BRAN/HANG tubing）
pub async fn gen_cata_instances(
    db_option: Arc<DbOptionExt>,
    target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<CateGenOutcome> {
    let local_al_map = Arc::new(DashMap::new());
    gen_cata_geos_inner(
        db_option,
        target_cata_map,
        Arc::new(DashMap::new()),
        sjus_map_arc,
        sender,
        local_al_map,
        true,
        false,
    )
    .await?
    .cate
    .ok_or_else(|| anyhow::anyhow!("cate outcome missing"))
}

/// 仅处理 BRAN/HANG tubing（不生成普通 CATE 几何体）
pub async fn gen_branch_tubi(
    db_option: Arc<DbOptionExt>,
    branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>,
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    local_al_map: Arc<DashMap<RefnoEnum, [CateAxisParam; 2]>>,
) -> anyhow::Result<BranchTubiOutcome> {
    gen_cata_geos_inner(
        db_option,
        Arc::new(DashMap::new()),
        branch_map,
        sjus_map_arc,
        sender,
        local_al_map,
        false,
        true,
    )
    .await?
    .branch
    .ok_or_else(|| anyhow::anyhow!("branch outcome missing"))
}

#[instrument(skip(db_option, target_cata_map, branch_map, sjus_map_arc, sender))]
async fn gen_cata_geos_inner(
    db_option: Arc<DbOptionExt>,
    target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
    branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>,
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    local_al_map: Arc<DashMap<RefnoEnum, [CateAxisParam; 2]>>,
    process_cata: bool,
    process_branch: bool,
) -> anyhow::Result<GenOutcome> {
    // Initialize Chrome tracing
    #[cfg(feature = "profile")]
    init_chrome_tracing()?;

    let total_t = Instant::now();
    // let mut handles = FuturesUnordered::new();
    let mut tubi_relates = vec![];
    let gen_mesh = db_option.inner.gen_mesh;
    // replace_mesh/regen-model 的核心诉求是“重建关系/mesh”（比如 inst_relate、mesh 文件），
    // 但 inst_info(ptset) 若已存在仍可复用，以避免重复的元件库几何生成。
    let replace_exist = db_option.inner.is_replace_mesh();
    let is_bran = branch_map.len() > 0;
    
    // tubi_info 收集容器: 组合键 "{cata_hash}_{arrive}_{leave}" -> TubiInfoData
    let tubi_info_map: Arc<DashMap<String, TubiInfoData>> = Arc::new(DashMap::new());

    // 用于收集总耗时的互斥锁
    let total_time_stats = Arc::new(Mutex::new(HashMap::new()));

    let db_time_fetch_keys = Instant::now();
    let all_unique_keys = Arc::new(
        target_cata_map
            .iter()
            .map(|x| x.cata_hash.clone())
            .collect::<Vec<_>>(),
    );

    let unique_cata_cnt = all_unique_keys.len();
    debug_model_debug!(
        "gen_cata_geos start: unique_cata_cnt={}, target_cata_map_len={}, branch_map_len={}",
        unique_cata_cnt,
        target_cata_map.len(),
        branch_map.len()
    );
    let mut batch_chunks_cnt = 4usize.min(unique_cata_cnt.max(1));
    let mut batch_size = (unique_cata_cnt + batch_chunks_cnt - 1) / batch_chunks_cnt;
    if batch_size == 0 {
        batch_size = 1;
    }
    let test_refno = db_option.get_test_refno();
    //如果只有一个元件，就不分块了
    if batch_size == 1 {
        batch_chunks_cnt = unique_cata_cnt;
    } else {
        batch_chunks_cnt = (unique_cata_cnt + batch_size - 1) / batch_size;
    }
    #[cfg(feature = "profile")]
    tracing::info!(
        unique_cata_cnt,
        batch_chunks_cnt,
        "Starting to process catalog models"
    );

    if process_cata && !all_unique_keys.is_empty() {
        for i in 0..batch_chunks_cnt {
            let all_unique_keys = all_unique_keys.clone();
            let target_cata_map = target_cata_map.clone();
            let sjus_map_clone = sjus_map_arc.clone();
            let local_al_map_clone = local_al_map.clone();
            let tubi_info_map_clone = tubi_info_map.clone();
            let sender = sender.clone();
            let total_time_stats = total_time_stats.clone();
            let batch_id = i + 1;

            #[cfg(feature = "profile")]
            tracing::info!(batch_id, "Starting batch processing");

            let start_idx = i * batch_size;
            if start_idx >= unique_cata_cnt {
                debug_model_debug!(
                    "[gen_cata_geos] 批次 {} 起始索引 {} 超出总长度 {}, 跳过",
                    batch_id,
                    start_idx,
                    unique_cata_cnt
                );
                continue;
            }
            let mut end_idx = start_idx + batch_size;
            if end_idx > unique_cata_cnt {
                end_idx = unique_cata_cnt;
            }
            #[cfg(feature = "profile")]
            tracing::info!(start_idx, end_idx, "Processing batch range");
            let mut shape_insts_data = ShapeInstancesData::default();
            if is_bran {
                shape_insts_data.fill_basic_shapes();
            }

            let mut db_time_get_named_attmap = 0;
            let mut db_time_get_world_transform = 0;
            let mut db_time_get_cat_refno = 0;
            let mut db_time_query_single = 0;
            let mut db_time_gen_single_geoms = 0;
            let mut db_time_get_generic_type = 0;
            let mut db_time_hash_lock = 0;
            let mut db_time_query_refnos = 0;

            for j in start_idx..end_idx {
                #[cfg(feature = "profile")]
                tracing::debug!(item_idx = j, "Processing item");

                let cata_hash = all_unique_keys[j].clone();
                if cata_hash == "0" {
                    continue;
                }
                let target_cata = target_cata_map.get(&cata_hash).unwrap();
                // 避免持有 DashMap guard 跨越 await（会拉长 shard 锁占用时间）。
                let target_exist_inst = target_cata.exist_inst;
                let target_group_refnos = target_cata.group_refnos.clone();
                let target_ptset = target_cata.ptset.clone();
                drop(target_cata);
                let mut process_refno = None;
                let mut ptset_map = None;
                debug_model_debug!(
                    "[cata_hash={}] exist_inst={}, group_refnos={:?}",
                    cata_hash,
                    target_exist_inst,
                    target_group_refnos
                );

                // regen-model/replace_mesh 场景：更偏向“重建并校验几何正确性”，
                // 若复用旧 inst_info/inst_geo，可能掩盖“同一 cata_hash 组内部分 refno 缺失实例”的问题。
                // 因此在 replace_mesh 模式下强制走生成路径，确保 group_refnos 都会被补齐。
                let force_regen_cata = process_cata && replace_exist;

                // 复用路径：inst_info 已存在（且 ptset 已可解析），跳过昂贵的元件库几何生成。
                // 注意：仍需为每个 pe 创建 inst_relate，并补齐 BRAN/HANG tubing 所需的 arrive/leave 点与伪属性缓存。
                if process_cata && target_exist_inst && !force_regen_cata {
                    debug_model_debug!(
                        "[cata_hash={}] reuse existing inst_info (skip gen_cata_single_geoms)",
                        cata_hash
                    );

                    let reuse_ptset_map = target_ptset.clone().unwrap_or_default();

                    // 伪属性缓存（用于 ATTRIB ... OF PREV 等表达式）；按 cata_hash 共享即可。
                    if let Some(&sample_refno) = target_group_refnos.first() {
                        if let Ok(sample_att) = aios_core::get_named_attmap(sample_refno).await {
                            if sample_att.contains_key("LEAV") {
                                let arrive = sample_att.get_i32("ARRI").unwrap_or_default();
                                let leave = sample_att.get_i32("LEAV").unwrap_or_default();
                                if let (Some(a), Some(l)) =
                                    (reuse_ptset_map.get(&arrive), reuse_ptset_map.get(&leave))
                                {
                                    let mut lock = HASH_PSEUDO_ATT_MAPS.write().await;
                                    let psudo_map = lock
                                        .entry(cata_hash.clone())
                                        .or_insert(NamedAttrMap::default());
                                    psudo_map.insert(
                                        "ARRWID".into(),
                                        NamedAttrValue::F32Type(a.pwidth),
                                    );
                                    psudo_map.insert(
                                        "ARRHEI".into(),
                                        NamedAttrValue::F32Type(a.pheight),
                                    );
                                    psudo_map.insert("ABOR".into(), NamedAttrValue::F32Type(a.pbore));
                                    psudo_map.insert(
                                        "LEAWID".into(),
                                        NamedAttrValue::F32Type(l.pwidth),
                                    );
                                    psudo_map.insert(
                                        "LEAHEI".into(),
                                        NamedAttrValue::F32Type(l.pheight),
                                    );
                                    psudo_map.insert("LBOR".into(), NamedAttrValue::F32Type(l.pbore));
                                }
                            }
                        }
                    }

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

                        let mut geos_info = EleGeosInfo {
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

                        if ele_att.contains_key("ARRI") && !reuse_ptset_map.is_empty() {
                            let arrive = ele_att.get_i32("ARRI").unwrap_or(-1);
                            let leave = ele_att.get_i32("LEAV").unwrap_or(-1);
                            if let (Some(a), Some(l)) =
                                (reuse_ptset_map.get(&arrive), reuse_ptset_map.get(&leave))
                            {
                                local_al_map_clone.insert(ele_refno, [a.clone(), l.clone()]);

                                // 仅对“有效 cata_hash”收集 tubi_info，避免 refno 兜底 key 进入数据库。
                                if is_valid_cata_hash(&cata_hash) {
                                    let tubi_info_id =
                                        TubiInfoData::make_id(&cata_hash, arrive, leave);
                                    tubi_info_map_clone
                                        .entry(tubi_info_id.clone())
                                        .or_insert_with(|| TubiInfoData::from_axis_params(&cata_hash, a, l));
                                    geos_info.tubi_info_id = Some(tubi_info_id);
                                }
                            }
                        }

                        shape_insts_data.insert_info(ele_refno, geos_info);
                        if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                            sender
                                .send(std::mem::take(&mut shape_insts_data))
                                .expect("send cate shape_insts_data error");
                        }
                    }
                    continue;
                }

                // inst_info 不存在：需要生成元件库几何（并产出 inst_info/inst_geo/geo_relate 等）。
                if process_cata && (force_regen_cata || !target_exist_inst) {
                    //如果没有已有的，需要生成
                    let ele_refno = match target_group_refnos.first().copied() {
                        Some(r) => r,
                        None => continue,
                    };
                    process_refno = Some(ele_refno);

                    let t_get_cat_refno = Instant::now();
                    #[cfg(feature = "profile")]
                    tracing::debug!(ele_refno = ?ele_refno, "Getting cat refno");
                    let result = aios_core::get_cat_refno(ele_refno).await;
                    let cata_refno = if let Ok(Some(refno)) = result {
                        debug_model_debug!(
                            "[cata_hash={}] ele_refno={} cat_refno={}",
                            cata_hash,
                            ele_refno,
                            refno
                        );
                        refno
                    } else {
                        debug_model_debug!(
                            "[WARN] get_cat_refno failed for ele_refno={} (result={:?})",
                            ele_refno,
                            result
                        );
                        use crate::fast_model::ModelErrorKind;
                        crate::model_error!(
                            code = "E-REF-001",
                            kind = ModelErrorKind::InvalidReference,
                            stage = "get_cat_refno",
                            refno = ele_refno,
                            desc = "获取元件库引用失败",
                            "ele_refno={}, result={:?}",
                            ele_refno,
                            result
                        );
                        #[cfg(feature = "profile")]
                        tracing::debug!(ele_refno = ?ele_refno, "元件库引用为空，跳过");
                        continue;
                    };
                    db_time_get_cat_refno += t_get_cat_refno.elapsed().as_millis();

                    #[cfg(feature = "profile")]
                    tracing::debug!(ele_refno = ?ele_refno, cata_refno = ?cata_refno, "开始生成元件库模型");

                    let t_query_single = Instant::now();
                    #[cfg(feature = "profile")]
                    tracing::debug!(cata_refno = ?cata_refno, "Querying GMSE");
                    let gmse_refno = aios_core::query_single_by_paths(
                        cata_refno,
                        &["->GMRE", "->GSTR"],
                        &["REFNO"],
                    )
                    .await
                    .map(|x| x.get_refno_or_default());
                    db_time_query_single += t_query_single.elapsed().as_millis();
                    match gmse_refno {
                        Ok(gmse) => {
                            debug_model_debug!(
                                "[cata_hash={}] ele_refno={} gmse_refno={}",
                                cata_hash,
                                ele_refno,
                                gmse
                            );
                        }
                        Err(err) => {
                            debug_model_debug!(
                                "[WARN] query_single_by_paths gmse_refno failed for ele_refno={}, cata_refno={}: {}",
                                ele_refno,
                                cata_refno,
                                err
                            );
                            use crate::fast_model::ModelErrorKind;
                            crate::model_error!(
                                code = "E-REF-002",
                                kind = ModelErrorKind::DbInconsistent,
                                stage = "query_gmse",
                                refno = ele_refno,
                                desc = "查询GMSE失败",
                                "ele_refno={}, cata_refno={}, err={}",
                                ele_refno,
                                cata_refno,
                                err
                            );
                            continue;
                        }
                    }

                    let t_query_single2 = Instant::now();
                    #[cfg(feature = "profile")]
                    tracing::debug!(cata_refno = ?cata_refno, "Querying NGMR");
                    let ngmr_refno =
                        aios_core::query_single_by_paths(cata_refno, &["->NGMR"], &["REFNO"])
                            .await
                            .map(|x| x.get_refno_or_default());
                    db_time_query_single += t_query_single2.elapsed().as_millis();

                    let valid_gmse = gmse_refno.as_ref().map(|r| r.is_valid()).unwrap_or(false);
                    let valid_ngmr = ngmr_refno.as_ref().map(|r| r.is_valid()).unwrap_or(false);

                    if !valid_gmse && !valid_ngmr {
                        use crate::fast_model::ModelErrorKind;
                        crate::model_error!(
                            code = "E-DATA-001",
                            kind = ModelErrorKind::DataMissing,
                            stage = "validate_gmse_ngmr",
                            refno = ele_refno,
                            desc = "GMSE和NGMR都无效",
                            "ele_refno={}, cata_refno={}",
                            ele_refno,
                            cata_refno
                        );
                        continue;
                    }

                    let csg_shapes_map = CateCsgShapeMap::new();
                    debug_model!(
                        "Created new csg_shapes_map for ele_refno={}, cata_hash={}",
                        ele_refno,
                        &cata_hash
                    );

                    let t_get_named_attmap = Instant::now();
                    #[cfg(feature = "profile")]
                    tracing::debug!(ele_refno = ?ele_refno, "Getting named attmap");
                    let desi_att = match aios_core::get_named_attmap(ele_refno).await {
                        Ok(att) => att,
                        Err(e) => {
                            record_refno_error(
                                RefnoErrorKind::NotFound,
                                RefnoErrorStage::Query,
                                "fast_model/cata_model.rs",
                                "get_named_attmap",
                                format!("DESI 属性获取失败: {}", e),
                                Some(&ele_refno),
                                None,
                                &[],
                                None,
                            );
                            continue;
                        }
                    };
                    db_time_get_named_attmap += t_get_named_attmap.elapsed().as_millis();

                    let mut design_axis_map = DashMap::new();
                    let cur_type = desi_att.get_type_str();
                    debug_model!(
                        "ele_refno={}, cur_type={}, valid_gmse={}, valid_ngmr={}",
                        ele_refno,
                        cur_type,
                        valid_gmse,
                        valid_ngmr
                    );

                    let t_gen_single_geoms = Instant::now();
                    #[cfg(feature = "profile")]
                    tracing::debug!(ele_refno = ?ele_refno, "Generating single geoms");
                    debug_model!("Calling gen_cata_single_geoms for ele_refno={}", ele_refno);
                    let r =
                        gen_cata_single_geoms(ele_refno, &csg_shapes_map, &design_axis_map).await;
                    db_time_gen_single_geoms += t_gen_single_geoms.elapsed().as_millis();
                    debug_model!(
                        "After gen_cata_single_geoms, csg_shapes_map.len()={}",
                        csg_shapes_map.len()
                    );
                    // dbg!(&csg_shapes_map);

                    match r {
                        Ok(_) => {
                            #[cfg(feature = "profile")]
                            tracing::debug!(ele_refno = ?ele_refno, "生成元件库模型成功");
                        }
                        Err(e) => {
                            #[cfg(feature = "profile")]
                            tracing::error!(ele_refno = ?ele_refno, error = ?e, "生成元件库模型失败");

                            // 根据错误信息分类
                            use crate::fast_model::ModelErrorKind;
                            let err_str = e.to_string().to_lowercase();
                            let (code, kind) = if err_str.contains("expression")
                                || err_str.contains("表达式")
                                || err_str.contains("resolve_cata_comp")
                                || err_str.contains("eval")
                            {
                                ("E-EXPR-001", ModelErrorKind::InvalidGeometry)
                            } else if err_str.contains("geometry")
                                || err_str.contains("profile")
                                || err_str.contains("polygon")
                            {
                                ("E-GEO-002", ModelErrorKind::InvalidGeometry)
                            } else {
                                ("E-PIPE-001", ModelErrorKind::PipelineError)
                            };

                            let desc = match code {
                                "E-EXPR-001" => "表达式计算失败",
                                "E-GEO-002" => "几何数据无效",
                                _ => "生成模型失败",
                            };

                            crate::model_error!(
                                code = code,
                                kind = kind,
                                stage = "gen_cata_single_geoms",
                                refno = ele_refno,
                                desc = desc,
                                "ele_refno={}, err={}",
                                ele_refno,
                                e
                            );
                            continue;
                        }
                    };

                    // 同一 cata_hash 可能对应多个 design_refno（group_refnos）。
                    // gen_cata_single_geoms 只会把几何写入当前 design_refno 的 key。
                    //
                    // 但导出/后续关系构建需要“每个 design_refno 都有一份 inst_geo/inst_info/ptset_map”。
                    // 因此这里将代表 refno 的 shapes 与轴点集复制到同组其它 refno。
                    //
                    // 说明：
                    // - 这些 shapes/ptset 都是“局部坐标系”下的数据；不同实例的世界变换会在后续循环中分别应用。
                    // - 这里用 clone 做最小侵入修复；group_refnos 通常很小，成本可接受。
                    if target_group_refnos.len() > 1 {
                        let shapes_tpl = csg_shapes_map.get(&ele_refno).map(|v| v.clone());
                        let axis_tpl = design_axis_map.get(&ele_refno).map(|v| v.clone());
                        if let (Some(shapes_tpl), Some(axis_tpl)) = (shapes_tpl, axis_tpl) {
                            for &other_refno in target_group_refnos.iter() {
                                if other_refno == ele_refno {
                                    continue;
                                }
                                // 若下游已写入（例如未来 gen_cata_single_geoms 改为批量写入），则不覆盖。
                                csg_shapes_map
                                    .entry(other_refno)
                                    .or_insert_with(|| shapes_tpl.clone());
                                design_axis_map
                                    .entry(other_refno)
                                    .or_insert_with(|| axis_tpl.clone());
                            }
                        }
                    }

                    {
                        // 将一些伪属性需要用到的值存下来，后面也要更新维护这些伪属性，避免重复计算
                        let t_lock = Instant::now();
                        let mut lock = HASH_PSEUDO_ATT_MAPS.write().await;
                        db_time_hash_lock += t_lock.elapsed().as_millis();

                        let psudo_map = lock
                            .entry(cata_hash.clone())
                            .or_insert(NamedAttrMap::default());

                        if desi_att.contains_key("LEAV") {
                            let arrive = desi_att.get_i32("ARRI").unwrap_or_default();
                            let leave = desi_att.get_i32("LEAV").unwrap_or_default();
                            let axis_map = design_axis_map.get(&ele_refno).unwrap();
                            if axis_map.contains_key(&arrive) {
                                let v = axis_map.get(&arrive).unwrap();
                                psudo_map
                                    .insert("ARRWID".into(), NamedAttrValue::F32Type(v.pwidth));
                                psudo_map
                                    .insert("ARRHEI".into(), NamedAttrValue::F32Type(v.pheight));
                                psudo_map.insert("ABOR".into(), NamedAttrValue::F32Type(v.pbore));
                            }

                            if axis_map.contains_key(&leave) {
                                let v = axis_map.get(&leave).unwrap();
                                psudo_map
                                    .insert("LEAWID".into(), NamedAttrValue::F32Type(v.pwidth));
                                psudo_map
                                    .insert("LEAHEI".into(), NamedAttrValue::F32Type(v.pheight));
                                psudo_map.insert("LBOR".into(), NamedAttrValue::F32Type(v.pbore));
                            }
                        }
                    }

                    /// 处理几何体的 shapes：按 group_refnos 逐个处理，确保同一 cata_hash 组内每个 refno 都被写入。
                    debug_model!(
                        "Processing csg_shapes_map, entries count: {}",
                        csg_shapes_map.len()
                    );
                    for &ele_refno in target_group_refnos.iter() {
                        let shapes = csg_shapes_map
                            .remove(&ele_refno)
                            .map(|x| x.1)
                            .unwrap_or_default();
                        debug_model!(
                            "Processing ele_refno={}, shapes.len()={}",
                            ele_refno,
                            shapes.len()
                        );
                        if shapes.is_empty() {
                            // 同组 refno 理论上应共享几何；若此处为空，说明上游生成/复制缺失，直接跳过即可。
                            continue;
                        }
                        let t_get_world_transform = Instant::now();
                        // 统一走 foyer transform_cache（不要求全量命中）；miss 时按需计算并回写缓存。
                        let mut world_transform = match crate::fast_model::transform_cache::get_world_transform_cache_first(
                            Some(db_option.as_ref()),
                            ele_refno,
                        )
                        .await
                        {
                            Ok(Some(t)) => t,
                            Ok(None) => {
                                debug_model!(
                                    "[SKIP] ele_refno={} world_transform 为 None，跳过",
                                    ele_refno
                                );
                                record_refno_error(
                                    RefnoErrorKind::Missing,
                                    RefnoErrorStage::Query,
                                    "fast_model/cata_model.rs",
                                    "get_world_transform_cache_first",
                                    "world_transform 为 None",
                                    Some(&ele_refno),
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
                                    "fast_model/cata_model.rs",
                                    "get_world_transform_cache_first",
                                    format!("获取/计算 world_transform 失败: {}", e),
                                    Some(&ele_refno),
                                    None,
                                    &[],
                                    None,
                                );
                                continue;
                            }
                        };
                        db_time_get_world_transform += t_get_world_transform.elapsed().as_millis();
                        debug_model!(
                            "[WORLD_TRANS_DBG] ele_refno={}, world_transform: rot={:?}, trans={:?}, scale={:?}",
                            ele_refno, world_transform.rotation, world_transform.translation, world_transform.scale
                        );

                        let t_get_named_attmap2 = Instant::now();
                        let ele_att = match aios_core::get_named_attmap(ele_refno).await {
                            Ok(att) => att,
                            Err(e) => {
                                record_refno_error(
                                    RefnoErrorKind::NotFound,
                                    RefnoErrorStage::Query,
                                    "fast_model/cata_model.rs",
                                    "get_named_attmap",
                                    format!("获取 named_attmap 失败: {}", e),
                                    Some(&ele_refno),
                                    None,
                                    &[],
                                    None,
                                );
                                continue;
                            }
                        };
                        db_time_get_named_attmap += t_get_named_attmap2.elapsed().as_millis();

                        if let Some(sjus) = ele_att.get_str("SJUS") {
                            let parent = ele_att.get_owner();
                            if let Some(sjus_adjust) = sjus_map_clone.get(&parent) {
                                let height = sjus_adjust.value().1;
                                let off_z = cal_sjus_value(sjus, height);

                                let t_get_world_transform2 = Instant::now();
                                let parent_trans = aios_core::get_world_mat4(parent, false)
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|mat4| {
                                        let (scale, rotation, translation) = mat4.to_scale_rotation_translation();
                                        bevy_transform::prelude::Transform {
                                            translation: translation.as_vec3(),
                                            rotation: glam::Quat::from_xyzw(
                                                rotation.x as f32, rotation.y as f32,
                                                rotation.z as f32, rotation.w as f32,
                                            ),
                                            scale: scale.as_vec3(),
                                        }
                                    })
                                    .unwrap_or_default();
                                db_time_get_world_transform +=
                                    t_get_world_transform2.elapsed().as_millis();

                                world_transform.translation.z = parent_trans.translation.z;
                                world_transform.translation = world_transform.translation
                                    + sjus_adjust.value().0
                                    + Vec3::new(0.0, 0.0, off_z);
                            }
                        }

                        //判断是否有负实体的集合组合，在这里做一个合并处理，只要发现有负实体，就合并在一起
                        //反过来查询负实体，然后查询它的owner，来找到相邻的正实体
                        let t_query_refnos = Instant::now();
                        let mut pos_neg_map: HashMap<RefnoEnum, Vec<RefnoEnum>> = if valid_gmse {
                            if let Ok(gmse) = &gmse_refno {
                                aios_core::query_refnos_has_pos_neg_map(&[*gmse], Some(true))
                                    .await
                                    .unwrap_or_default()
                            } else {
                                HashMap::new()
                            }
                        } else {
                            HashMap::new()
                        };
                        db_time_query_refnos += t_query_refnos.elapsed().as_millis();

                        let mut neg_own_pos_map: HashMap<RefnoEnum, RefnoEnum> = pos_neg_map
                            .iter()
                            .map(|(k, negs)| negs.iter().map(|x| (*x, *k)))
                            .flatten()
                            .collect();

                        let cur_ptset_map = design_axis_map
                            .remove(&ele_refno)
                            .map(|x| x.1)
                            .unwrap_or_default();

                        let t_get_generic_type = Instant::now();
                        let generic_type = get_generic_type(ele_refno).await.unwrap_or_default();
                        db_time_get_generic_type += t_get_generic_type.elapsed().as_millis();

                        let (owner_refno, owner_type) =
                            shared::get_owner_info_from_attr(&ele_att).await;
                        let cata_hash_for_info = if is_valid_cata_hash(&cata_hash) {
                            Some(cata_hash.clone())
                        } else {
                            None
                        };
                        let mut geos_info = EleGeosInfo {
                            refno: ele_refno,
                            sesno: ele_att.sesno(),
                            owner_refno,
                            owner_type,
                            cata_hash: cata_hash_for_info,
                            visible: true,
                            generic_type,
                            aabb: None,
                            world_transform,
                            cata_refno: Some(cata_refno),
                            ptset_map: cur_ptset_map.clone(),
                            is_solid: true,
                            ..Default::default()
                        };

                        if ele_att.contains_key("ARRI") && !cur_ptset_map.is_empty() {
                            let arrive = ele_att.get_i32("ARRI").unwrap_or(-1);
                            let leave = ele_att.get_i32("LEAV").unwrap_or(-1);
                            if let Some(a) = cur_ptset_map.values().find(|x| x.number == arrive)
                                && let Some(l) = cur_ptset_map.values().find(|x| x.number == leave)
                            {
                                local_al_map_clone.insert(ele_refno, [a.clone(), l.clone()]);
                                
                                // 收集 tubi_info（增量，自动去重）
                                if is_valid_cata_hash(&cata_hash) {
                                    let tubi_info_id = TubiInfoData::make_id(&cata_hash, arrive, leave);
                                    tubi_info_map_clone.entry(tubi_info_id.clone()).or_insert_with(|| {
                                        TubiInfoData::from_axis_params(&cata_hash, a, l)
                                    });
                                    geos_info.tubi_info_id = Some(tubi_info_id);
                                }
                            }
                            ptset_map = Some(cur_ptset_map);
                        };

                        let mut geo_insts = vec![];
                        // 诊断：为什么最终 inst_cnt=0（通常是所有 shape 都在这里被跳过）
                        let mut shape_total = 0usize;
                        let mut shape_skip_invalid = 0usize;
                        let mut shape_skip_invisible = 0usize;
                        let mut shape_skip_nan = 0usize;
                        let mut shape_added = 0usize;
                        let mut visible_set = HashSet::new();
                        for s in &shapes {
                            if s.visible {
                                visible_set.insert(s.refno);
                            }
                        }

                        debug_model!(
                            "About to process {} shapes for ele_refno={}",
                            shapes.len(),
                            ele_refno
                        );

                        for (shape_idx, shape) in shapes.into_iter().enumerate() {
                            shape_total += 1;
                            debug_model!(
                                "Processing shape[{}] for ele_refno={}",
                                shape_idx,
                                ele_refno
                            );
                            let CateCsgShape {
                                refno: geom_refno,
                                csg_shape,
                                transform,
                                visible,
                                is_tubi,
                                pts,
                                is_ngmr,
                                ..
                            } = shape;

                            if !csg_shape.check_valid() {
                                debug_model!(
                                    "shape[{}] csg_shape.check_valid() failed, skipping",
                                    shape_idx
                                );
                                shape_skip_invalid += 1;
                                continue;
                            }
                            if !visible {
                                debug_model!("shape[{}] not visible, skipping", shape_idx);
                                shape_skip_invisible += 1;
                                continue;
                            }
                            let mut shape_trans = csg_shape.get_trans();
                            let is_neg = neg_own_pos_map.contains_key(&geom_refno);
                            let geo_hash = csg_shape.hash_unit_mesh_params();
                            debug_model!(
                                "[TRANSFORM_DBG] ele_refno={}, geom_refno={}, geo_hash={}",
                                ele_refno, geom_refno, geo_hash
                            );
                            debug_model!(
                                "[TRANSFORM_DBG] CateCsgShape.transform: rot={:?}, trans={:?}, scale={:?}",
                                transform.rotation, transform.translation, transform.scale
                            );
                            debug_model!(
                                "[TRANSFORM_DBG] shape_trans (from get_trans): rot={:?}, trans={:?}, scale={:?}",
                                shape_trans.rotation, shape_trans.translation, shape_trans.scale
                            );
                            let rot = transform.rotation;
                            let translation = transform.translation
                                + transform.rotation * shape_trans.translation;
                            let scale = shape_trans.scale;
                            let mut transform = Transform {
                                translation,
                                rotation: rot,
                                scale,
                            };
                            debug_model!(
                                "[TRANSFORM_DBG] final transform: rot={:?}, trans={:?}, scale={:?}",
                                transform.rotation, transform.translation, transform.scale
                            );
                            if transform.translation.is_nan()
                                || transform.rotation.is_nan()
                                || transform.scale.is_nan()
                            {
                                debug_model!(
                                    "shape[{}] transform contains NaN, skipping",
                                    shape_idx
                                );
                                shape_skip_nan += 1;
                                continue;
                            }
                            let mut cata_neg_refnos =
                                pos_neg_map.remove(&geom_refno).unwrap_or_default();
                            cata_neg_refnos.retain(|x| visible_set.contains(x));
                            if !cata_neg_refnos.is_empty() {
                                geos_info.has_cata_neg = true;
                            }
                            let geo_type = if is_ngmr {
                                GeoBasicType::CataCrossNeg
                            } else if is_neg {
                                GeoBasicType::CataNeg
                            } else if !cata_neg_refnos.is_empty() {
                                GeoBasicType::Compound
                            } else {
                                // 初始正实体，布尔运算时会被改为 CatePos
                                GeoBasicType::Pos
                            };
                            let mut geo_param = csg_shape
                                .convert_to_geo_param()
                                .unwrap_or(PdmsGeoParam::Unknown);
                            let unit_flag = csg_shape.is_reuse_unit();

                            // unit_flag=true 时，写入"单位参数"，避免同一 geo_hash 复用时 mesh 被绝对尺寸污染，
                            // 同时确保保留 transform.scale 时不会重复缩放。
                            if unit_flag {
                                geo_param = csg_shape
                                    .gen_unit_shape()
                                    .convert_to_geo_param()
                                    .unwrap_or(geo_param);
                            }

                            // 统一处理 transform.scale 清零逻辑
                            crate::fast_model::reuse_unit::normalize_transform_scale(
                                &mut transform,
                                unit_flag,
                                geo_hash,
                            );
                            let geom_inst = EleInstGeo {
                                geo_hash,
                                refno: geom_refno,
                                pts,
                                aabb: None,
                                geo_transform: transform,
                                geo_param,
                                visible: geo_type == GeoBasicType::Pos
                                    || geo_type == GeoBasicType::Compound,
                                is_tubi,
                                geo_type,
                                cata_neg_refnos: cata_neg_refnos.clone(),
                            };

                            // 将 CATE 的负实体关系写入 neg_relate_map
                            // 这样可以统一 LOOP/PRIM/CATE 的负实体存储方式
                            if !cata_neg_refnos.is_empty() {
                                shape_insts_data.insert_negs(geom_refno, &cata_neg_refnos);
                            }

                            if is_ngmr {
                                if let Ok(target_owners) =
                                    query_ngmr_owner(ele_refno, geom_refno).await
                                {
                                        shape_insts_data.insert_ngmr(
                                            ele_refno,
                                            target_owners,
                                            geom_refno,
                                        );
                                    }
                            }
                            debug_model!("shape[{}] successfully added to geo_insts", shape_idx);
                            shape_added += 1;
                            geo_insts.push(geom_inst);
                        }
                        {
                            debug_model!(
                                "Finished processing shapes, geo_insts.len()={}",
                                geo_insts.len()
                            );
                            debug_model!(
                                "Shape stats: total={}, added={}, skip_invalid={}, skip_invisible={}, skip_nan={}",
                                shape_total,
                                shape_added,
                                shape_skip_invalid,
                                shape_skip_invisible,
                                shape_skip_nan
                            );
                            let mut inst_key = geos_info.get_inst_key();
                            geos_info.is_solid = geo_insts.iter().any(|x| {
                                x.geo_type == GeoBasicType::Pos
                                    || x.geo_type == GeoBasicType::Compound
                            });
                            let mut geos_data = EleInstGeosData {
                                inst_key,
                                refno: ele_refno,
                                insts: geo_insts,
                                aabb: None,
                                type_name: cur_type.to_string(),
                                ..Default::default()
                            };
                            if geos_data.insts.len() > 0 {
                                debug_model!(
                                    "Inserting geos_data for ele_refno={}, insts.len()={}",
                                    ele_refno,
                                    geos_data.insts.len()
                                );
                                shape_insts_data.insert_info(ele_refno, geos_info.clone());
                                shape_insts_data
                                    .insert_geos_data(geos_info.get_inst_key(), geos_data);
                            } else {
                                debug_model_debug!(
                                    "[WARN] geos_data.insts is empty, NOT inserting for ele_refno={}",
                                    ele_refno
                                );
                            }
                        }
                    }
                }
                for ele_refno in target_group_refnos.clone() {
                    if Some(ele_refno) == process_refno {
                        continue;
                    }
                    let cur_ptset_map = ptset_map
                        .as_ref()
                        .or(target_ptset.as_ref())
                        .cloned()
                        .unwrap_or_default();
                    // 统一走 foyer transform_cache（不要求全量命中）；miss 时按需计算并回写缓存。
                    let mut origin_trans =
                        match crate::fast_model::transform_cache::get_world_transform_cache_first(
                            Some(db_option.as_ref()),
                            ele_refno,
                        )
                        .await
                        {
                            Ok(Some(t)) => t,
                            Ok(None) => {
                                debug_model_debug!(
                                    "[WARN] world_transform 为 None, ele_refno={}, cata_hash={}, 跳过该元件",
                                    ele_refno,
                                    cata_hash
                                );
                                continue;
                            }
                            Err(e) => {
                                debug_model_debug!(
                                    "[WARN] 获取/计算 world_transform 失败, ele_refno={}, cata_hash={}, error={}, 跳过该元件",
                                    ele_refno,
                                    cata_hash,
                                    e
                                );
                                continue;
                            }
                        };

                    let ele_att = aios_core::get_named_attmap(ele_refno)
                        .await
                        .unwrap_or_default();
                    if let Some(sjus) = ele_att.get_str("SJUS") {
                        let parent = ele_att.get_owner();
                        if let Some(sjus_adjust) = sjus_map_clone.get(&parent) {
                            let height = sjus_adjust.value().1;
                            let off_z = cal_sjus_value(sjus, height);
                            origin_trans.translation += sjus_adjust.value().0
                                + origin_trans.rotation * Vec3::new(0.0, 0.0, off_z);
                        }
                    }

                    // 收集 arrive/leave 点信息
                    let mut tubi_info_id = None;
                    if ele_att.contains_key("ARRI") && !cur_ptset_map.is_empty() {
                        let arrive = ele_att.get_i32("ARRI").unwrap_or(-1);
                        let leave = ele_att.get_i32("LEAV").unwrap_or(-1);
                        if let Some(a) = cur_ptset_map.values().find(|x| x.number == arrive)
                            && let Some(l) = cur_ptset_map.values().find(|x| x.number == leave)
                        {
                            local_al_map_clone.insert(ele_refno, [a.clone(), l.clone()]);
                            
                            // 收集 tubi_info（增量，自动去重）
                            let id = TubiInfoData::make_id(&cata_hash, arrive, leave);
                            tubi_info_map_clone.entry(id.clone()).or_insert_with(|| {
                                TubiInfoData::from_axis_params(&cata_hash, a, l)
                            });
                            tubi_info_id = Some(id);
                        }
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
                        generic_type: get_generic_type(ele_refno).await.unwrap_or_default(),
                        world_transform: origin_trans,
                        ptset_map: cur_ptset_map,
                        is_solid: true,
                        tubi_info_id,
                        ..Default::default()
                    };
                    if let Some(r_refno) = test_refno
                        && r_refno == ele_refno
                    {
                        tracing::debug!("{:?}", &geos_info);
                    }
                    shape_insts_data.insert_info(ele_refno, geos_info);
                }
                if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                    #[cfg(feature = "profile")]
                    tracing::info!(
                        batch_id,
                        items_cnt = shape_insts_data.inst_cnt(),
                        "Sending batch data due to size limit"
                    );

                    sender
                        .send(std::mem::take(&mut shape_insts_data))
                        .expect("send cate shape_insts_data error");
                }
            }

            // 将本批次的时间统计添加到总时间统计中
            #[cfg(feature = "profile")]
            {
                let mut stats = total_time_stats.lock().await;
                *stats.entry("get_named_attmap".to_string()).or_insert(0) +=
                    db_time_get_named_attmap as u64;
                *stats.entry("get_world_transform".to_string()).or_insert(0) +=
                    db_time_get_world_transform as u64;
                *stats.entry("get_cat_refno".to_string()).or_insert(0) +=
                    db_time_get_cat_refno as u64;
                *stats.entry("query_single".to_string()).or_insert(0) +=
                    db_time_query_single as u64;
                *stats.entry("gen_single_geoms".to_string()).or_insert(0) +=
                    db_time_gen_single_geoms as u64;
                *stats.entry("get_generic_type".to_string()).or_insert(0) +=
                    db_time_get_generic_type as u64;
                *stats.entry("hash_lock".to_string()).or_insert(0) += db_time_hash_lock as u64;
                *stats.entry("query_refnos".to_string()).or_insert(0) +=
                    db_time_query_refnos as u64;
            }

            if shape_insts_data.inst_cnt() > 0 {
                debug_model!(
                    "Sending shape_insts_data at end of batch, inst_cnt={}",
                    shape_insts_data.inst_cnt()
                );
                sender
                    .send(shape_insts_data)
                    .expect("send prim shape_insts_data error");
            } else {
                debug_model!("shape_insts_data.inst_cnt() is 0, NOT sending");
            }

            #[cfg(feature = "profile")]
            tracing::info!(batch_id, "Batch processing complete");
        }
    }

    #[cfg(feature = "profile")]
    tracing::info!("Waiting for batches to complete");

    // Wait for batches to complete
    // while let Some(_) = handles.next().await {}

    let mut process_branch_time: u128 = 0;
    let mut db_time_get_children = 0;
    let mut db_time_get_branch_att = 0;
    let mut db_time_get_branch_transform = 0;
    let mut tubi_count = 0;
    let mut send_data_time = 0;
    let mut tubi_query_time = 0;

    let mut tubi_refnos: Vec<String> = Vec::new();
    let mut tubi_pts_map: DashMap<u64, String> = DashMap::new();
    // tubi_relate 依赖的 trans/aabb/vec3 记录收集器：
    // debug-model / cache-only 场景下，可能会写入 tubi_relate，但 save_db=false 导致 trans/aabb/vec3 未落库，
    // 从而出现 world_trans/start_pt/end_pt 悬空（导出/调试查询拿到 NONE）。
    let mut tubi_trans_map: HashMap<u64, String> = HashMap::new();
    let tubi_aabb_map: DashMap<String, Aabb> = DashMap::new();
    if process_branch {
        #[cfg(feature = "profile")]
        tracing::info!(
            branch_count = branch_map.len(),
            "Processing branches (BRAN Tubing generation)"
        );
        let unit_cyli_aabb = Aabb::new(Point3::new(-0.5, -0.5, 0.0), Point3::new(0.5, 0.5, 1.0));
        let mut tubi_shape_insts_data = ShapeInstancesData::default();

        let t_process_branch = Instant::now();

        for bran_data in branch_map.iter() {
            let branch_refno = *bran_data.key();
            let children = bran_data.value();
            let is_debug_branch = branch_refno.to_string() == "24381_103385"
                || branch_refno.to_e3d_id() == "24381/103385";

            debug_model!(
                "[BRAN_TUBI] 开始处理 BRAN/HANG 分支: refno={}, children_len={}",
                branch_refno.to_string(),
                children.len()
            );

            #[cfg(feature = "profile")]
            let branch_item_start = Instant::now();

            let t_get_children = Instant::now();
            // let Ok(children) = aios_core::get_children_pes(branch_refno).await else {
            //     continue;
            // };
            db_time_get_children += t_get_children.elapsed().as_millis();

            let t_get_named_attmap = Instant::now();
            let branch_att = match aios_core::get_named_attmap(branch_refno).await {
                Ok(att) => att,
                Err(e) => {
                    record_refno_error(
                        RefnoErrorKind::NotFound,
                        RefnoErrorStage::Query,
                        "fast_model/cata_model.rs",
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
            db_time_get_branch_att += t_get_named_attmap.elapsed().as_millis();

            let t_get_world_transform = Instant::now();
            let branch_transform =
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
                            "fast_model/cata_model.rs",
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
                            "fast_model/cata_model.rs",
                            "get_world_transform_cache_first",
                            format!("BRAN/HANG 获取/计算 world_transform 失败: {}", e),
                            Some(&branch_refno),
                            None,
                            &[],
                            None,
                        );
                        continue;
                    }
                };
            db_time_get_branch_transform += t_get_world_transform.elapsed().as_millis();

            let Some(hpt) = branch_att.get_vec3("HPOS") else {
                record_refno_error(
                    RefnoErrorKind::Missing,
                    RefnoErrorStage::Build,
                    "fast_model/cata_model.rs",
                    "branch_hpos",
                    "BRAN/HANG 缺少 HPOS",
                    Some(&branch_refno),
                    None,
                    &[],
                    None,
                );
                continue;
            };
            let htube_pt = branch_transform.transform_point(hpt);
            let hdir = branch_transform
                .to_matrix()
                .transform_vector3(branch_att.get_vec3("HDIR").unwrap())
                .normalize_or_zero();
            let bran_ttube_pt =
                branch_transform.transform_point(branch_att.get_vec3("TPOS").unwrap());

            let is_hang = branch_att.get_type_str() == "HANG";
            let h_ref = branch_att
                .get_foreign_refno(if is_hang { "HREF" } else { "HSTU" })
                .unwrap_or_default();

            let tubi_att = aios_core::get_named_attmap(h_ref).await.unwrap_or_default();
            let tubi_cat_ref = tubi_att.get_foreign_refno("CATR").unwrap_or_default();
            let mut h_tubi_size =
                fast_model::query_tubi_size(branch_refno, tubi_cat_ref, is_hang).await?;
            let mut tubi_geo_hash = if matches!(h_tubi_size, TubiSize::BoxSize(_)) {
                BOXI_GEO_HASH
            } else {
                TUBI_GEO_HASH
            };

            let tref = branch_att
                .get_foreign_refno(if is_hang { "TREF" } else { "LSTU" })
                .unwrap_or_default();
            let tdir = branch_transform
                .to_matrix()
                .transform_vector3(branch_att.get_vec3("TDIR").unwrap())
                .normalize_or_zero();
            let mut current_tubing = PdmsTubing {
                leave_refno: branch_refno,
                arrive_refno: tref,
                start_pt: htube_pt,
                end_pt: Vec3::ZERO,
                desire_leave_dir: hdir,
                leave_ref_dir: None,
                desire_arrive_dir: Default::default(),
                tubi_size: h_tubi_size,
                index: 0,
            };

            let bran_owner_type = aios_core::get_type_name(branch_att.get_owner())
                .await
                .unwrap_or_default();
            let is_hvac = bran_owner_type == "HVAC";
            if children.len() == 0 && !is_hvac {
                let dist = bran_ttube_pt.distance(current_tubing.start_pt);
                current_tubing.arrive_refno = tref;
                current_tubing.end_pt = bran_ttube_pt;
                current_tubing.desire_arrive_dir = tdir;
                let actual_dir = (current_tubing.end_pt - current_tubing.start_pt).normalize_or_zero();
                if actual_dir.length_squared() > 0.0
                    && current_tubing.desire_arrive_dir.dot(actual_dir) < 0.99
                {
                    current_tubing.desire_arrive_dir = actual_dir;
                }
                debug_model!(
                    "[BRAN_TUBI] 末端直段候选(无子元素): bran_refno={}, start={}, end={}, dist={:.3}, desire_arrive_dir={}",
                    branch_refno.to_string(),
                    to_pdms_vec_str(&current_tubing.start_pt, false),
                    to_pdms_vec_str(&current_tubing.end_pt, false),
                    dist,
                    to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                );
                let dir_ok = current_tubing.is_dir_ok();
                let dist_ok = dist > TUBI_TOL;
                let transform = (if !dir_ok {
                    build_tubi_transform_from_segment(
                        current_tubing.start_pt,
                        current_tubing.end_pt,
                        &current_tubing.tubi_size,
                    )
                } else {
                    current_tubing.get_transform()
                })
                .or_else(|| {
                    debug_model!(
                        "[BRAN_TUBI] 无法计算 transform (无子元素), 使用 fallback transform"
                    );
                    let mut fallback = Transform::IDENTITY;
                    fallback.translation = current_tubing.start_pt;
                    Some(fallback)
                });
                if let Some(t) = transform {
                    if is_debug_branch {
                        debug_model!(
                            "[BRAN_TUBI][DBG][no-children] leave={} arrive={} idx={} dist={:.3} dir_ok={} start={} end={} dir={}",
                            current_tubing.leave_refno.to_e3d_id(),
                            current_tubing.arrive_refno.to_e3d_id(),
                            current_tubing.index,
                            dist,
                            dir_ok,
                            to_pdms_vec_str(&current_tubing.start_pt, false),
                            to_pdms_vec_str(&current_tubing.end_pt, false),
                            to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                        );
                        debug_model!(
                            "[BRAN_TUBI][DBG][no-children] trans=({:.3},{:.3},{:.3}) scale=({:.3},{:.3},{:.3}) rot=({:.6},{:.6},{:.6},{:.6})",
                            t.translation.x, t.translation.y, t.translation.z,
                            t.scale.x, t.scale.y, t.scale.z,
                            t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                        );
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open("output/tubi_dbg.txt")
                        {
                            let _ = writeln!(
                                f,
                                "[no-children] leave={} arrive={} idx={} dist={:.3} dir_ok={} start={} end={} dir={} trans=({:.3},{:.3},{:.3}) scale=({:.3},{:.3},{:.3}) rot=({:.6},{:.6},{:.6},{:.6})",
                                current_tubing.leave_refno.to_e3d_id(),
                                current_tubing.arrive_refno.to_e3d_id(),
                                current_tubing.index,
                                dist,
                                dir_ok,
                                to_pdms_vec_str(&current_tubing.start_pt, false),
                                to_pdms_vec_str(&current_tubing.end_pt, false),
                                to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                                t.translation.x, t.translation.y, t.translation.z,
                                t.scale.x, t.scale.y, t.scale.z,
                                t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                            );
                        }
                    }
                    if dist_ok {
                        let aabb = shared::aabb_apply_transform(&unit_cyli_aabb, &t);
                        let aabb_hash = gen_aabb_hash(&aabb);
                        // Aabb 可能用于后续写入 inst_tubi_map，因此这里用 clone 避免移动。
                        tubi_aabb_map
                            .entry(aabb_hash.to_string())
                            .or_insert_with(|| aabb.clone());
                        let trans_hash = gen_bevy_transform_hash(&t);
                        tubi_trans_map.entry(trans_hash).or_insert_with(|| {
                            serde_json::to_string(&t).unwrap_or_else(|_| "null".to_string())
                        });
                        // 对于 tubing，owner 应该是 BRAN/HANG 本身，而不是 BRAN 的 owner
                        let owner_refno = branch_refno;
                        let owner_type = branch_att.get_type_str().to_string();
                        tubi_shape_insts_data.insert_tubi(
                            current_tubing.leave_refno,
                            EleGeosInfo {
                                refno: current_tubing.leave_refno,
                                sesno: branch_att.sesno(),
                                owner_refno,
                                owner_type,
                                cata_hash: Some(tubi_geo_hash.to_string()),
                                visible: true,
                                generic_type: get_generic_type(current_tubing.leave_refno).await.unwrap_or_default(),
                                aabb: Some(aabb),
                                world_transform: t,
                                tubi_start_pt: Some(current_tubing.start_pt),
                                tubi_end_pt: Some(current_tubing.end_pt),
                                // 无子元素直段：leave=htube_pt, arrive=bran_ttube_pt
                                arrive_axis_pt: Some(bran_ttube_pt.to_array()),
                                leave_axis_pt: Some(htube_pt.to_array()),
                                flow_pt_indexs: vec![],
                                cata_refno: None,
                                is_solid: true,
                                ..Default::default()
                            },
                        );
                        let start_hash = RsVec3(current_tubing.start_pt).gen_hash();
                        if !tubi_pts_map.contains_key(&start_hash) {
                            if let Ok(serialized) = serde_json::to_string(&RsVec3(current_tubing.start_pt)) {
                                tubi_pts_map.insert(start_hash, serialized);
                            }
                        }
                        let end_hash = RsVec3(current_tubing.end_pt).gen_hash();
                        if !tubi_pts_map.contains_key(&end_hash) {
                            if let Ok(serialized) = serde_json::to_string(&RsVec3(current_tubing.end_pt)) {
                                tubi_pts_map.insert(end_hash, serialized);
                            }
                        }

                        let arrive_hash = RsVec3(bran_ttube_pt).gen_hash();
                        if !tubi_pts_map.contains_key(&arrive_hash) {
                            if let Ok(serialized) = serde_json::to_string(&RsVec3(bran_ttube_pt)) {
                                tubi_pts_map.insert(arrive_hash, serialized);
                            }
                        }
                        let leave_hash = RsVec3(htube_pt).gen_hash();
                        if !tubi_pts_map.contains_key(&leave_hash) {
                            if let Ok(serialized) = serde_json::to_string(&RsVec3(htube_pt)) {
                                tubi_pts_map.insert(leave_hash, serialized);
                            }
                        }
                        tubi_relates.push(format!(
                        "relate {}->tubi_relate:[{}, {}]->{}  \
                        set geo=inst_geo:⟨{tubi_geo_hash}⟩,aabb=aabb:⟨{aabb_hash}⟩,world_trans=trans:⟨{trans_hash}⟩, start_pt=vec3:⟨{start_hash}⟩, end_pt=vec3:⟨{end_hash}⟩, arrive_axis=vec3:⟨{arrive_hash}⟩, leave_axis=vec3:⟨{leave_hash}⟩, bore_size={}, bad=false, system={}, dt=fn::ses_date({});",
                        current_tubing.leave_refno.to_pe_key(),
                        branch_refno.to_pe_key(),
                        current_tubing.index,
                        current_tubing.arrive_refno.to_pe_key(),
                        current_tubing.tubi_size.to_string(),
                        owner_refno.to_pe_key(),
                        current_tubing.leave_refno.to_pe_key(),
                    ));
                        current_tubing.index += 1;
                        tubi_count += 1;
                    } else {
                        debug_model!(
                            "[BRAN_TUBI] 跳过无子元素直段（距离过短）: dist={:.3}, TUBI_TOL={}",
                            dist,
                            TUBI_TOL
                        );
                    }
                }
                continue;
            }

            let mut bran_comp_vec = vec![];
            let len = children.len();
            let exist_refnos = children
                .iter()
                .map(|x| x.refno)
                .filter(|x| !local_al_map.contains_key(x))
                .collect::<Vec<_>>();
            let refus: Vec<RefU64> = exist_refnos.iter().map(|x| x.refno()).collect();
            let exist_al_map = aios_core::query_arrive_leave_points_of_component(&refus[..])
                .await
                .unwrap_or_default();
            debug_model!(
                "[BRAN_TUBI] exist_al_map 获取: total={}, 来自 db 查询={}, local_al_map_missing={}",
                exist_al_map.len(),
                refus.len(),
                exist_refnos.len()
            );

            // 🚀 从 cache 获取 ptset_map（当 exist_al_map 和 local_al_map 都没有时）
            let cache_al_map: DashMap<RefnoEnum, [CateAxisParam; 2]> = {
                let missing_refnos: Vec<RefnoEnum> = exist_refnos
                    .iter()
                    .filter(|r| !exist_al_map.contains_key(*r) && !local_al_map.contains_key(*r))
                    .copied()
                    .collect();
                if missing_refnos.is_empty() {
                    DashMap::new()
                } else if let Some(dbnum) = db_meta().get_dbnum_by_refno(branch_refno) {
                    let cache_dir = db_option.get_foyer_cache_dir();
                    if let Ok(cache) = InstanceCacheManager::new(&cache_dir).await {
                        let hm = cache.get_ptset_maps_for_refnos(dbnum, &missing_refnos).await;
                        let dm = DashMap::new();
                        for (k, v) in hm {
                            dm.insert(k, v);
                        }
                        dm
                    } else {
                        DashMap::new()
                    }
                } else {
                    DashMap::new()
                }
            };
            debug_model!(
                "[BRAN_TUBI] cache_al_map 获取: total={}",
                cache_al_map.len()
            );

            // 🚀 批量预取所有子元素的 world_transform（并发优化）
            let t_prefetch = Instant::now();
            let child_refnos: Vec<RefnoEnum> = children.iter().map(|x| x.refno).collect();
            let prefetch_transforms: HashMap<RefnoEnum, Transform> = {
                let mut futures = FuturesUnordered::new();
                for &refno in &child_refnos {
                    let db_option = db_option.clone();
                    futures.push(async move {
                        let trans = crate::fast_model::transform_cache::get_world_transform_cache_first(
                            Some(db_option.as_ref()),
                            refno,
                        )
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                        (refno, trans)
                    });
                }
                let mut result = HashMap::new();
                while let Some((refno, trans)) = futures.next().await {
                    result.insert(refno, trans);
                }
                result
            };
            debug_model!(
                "[BRAN_TUBI] 批量预取 world_transform: count={}, elapsed={}ms",
                prefetch_transforms.len(),
                t_prefetch.elapsed().as_millis()
            );

            let mut leave_type = "BRAN".to_string();
            let branch_tubi_before: i32 = tubi_count;
            for (index, ele) in children.into_iter().enumerate() {
                let refno = ele.refno;
                let arrive_type = ele.noun.as_str();
                let exclude = (is_hvac && index == 0);
                debug_model!(
                    "[BRAN_TUBI] 子件 {} [{}] -> arrive_type={}, exclude={}",
                    refno.to_e3d_id(),
                    index + 1,
                    arrive_type,
                    exclude
                );
                {
                    // 从预取缓存中获取 world_transform（无需 await）
                    let world_trans = prefetch_transforms
                        .get(&refno)
                        .cloned()
                        .unwrap_or_default();
                    let raw_axis = exist_al_map
                        .get(&refno)
                        .or(local_al_map.get(&refno))
                        .or(cache_al_map.get(&refno));
                    if raw_axis.is_none() {
                        debug_model!(
                            "[BRAN_TUBI] 子件 {} 无 axis_map (exist={}, local={}, cache={})",
                            refno.to_e3d_id(),
                            exist_al_map.get(&refno).is_some(),
                            local_al_map.get(&refno).is_some(),
                            cache_al_map.get(&refno).is_some()
                        );
                    }
                    if let Some(axis_map) = raw_axis.map(|x| {
                        [
                            x[0].transformed(&world_trans),
                            x[1].transformed(&world_trans),
                        ]
                    }) {
                        debug_model!(
                            "[BRAN_TUBI] 子件 {} axis_map arrive_pt={}, leave_pt={}",
                            refno.to_e3d_id(),
                            to_pdms_vec_str(&axis_map[0].pt, false),
                            to_pdms_vec_str(&axis_map[1].pt, false)
                        );
                        bran_comp_vec.push(refno);
                        current_tubing.arrive_refno = refno;
                        let mut skip = (arrive_type == "ATTA"
                            || arrive_type == "STIF"
                            || arrive_type == "BRCO")
                            && !aios_core::get_named_attmap(refno)
                                .await?
                                .get_bool_or_default("SPKBRK");
                        if !skip {
                            let a_pos = &axis_map[0].pt;
                            let Some(ref a_dir) = axis_map[0].dir else {
                                continue;
                            };

                            let actual_vec = **a_pos - current_tubing.start_pt;
                            let actual_dir = actual_vec.normalize_or_zero();
                            let same_dir = actual_dir.dot(**a_dir) > 0.99;
                            if same_dir {
                                debug_model!(
                                    "[BRAN_TUBI] actual_dir 与 a_dir 几乎同向: actual_dir={}, a_dir={}, refno={}",
                                    to_pdms_vec_str(&actual_dir, false),
                                    to_pdms_vec_str(&a_dir, false),
                                    refno.to_string()
                                );
                            } else {
                                debug_model!(
                                    "[BRAN_TUBI] 直段候选: leave_refno={}, arrive_refno={}, dist={:.3}, same_dir={}",
                                    current_tubing.leave_refno.to_e3d_id(),
                                    refno.to_e3d_id(),
                                    actual_vec.length(),
                                    same_dir
                                );
                            }
                            current_tubing.end_pt = **a_pos;
                            current_tubing.desire_arrive_dir = if same_dir {
                                **a_dir
                            } else {
                                actual_dir
                            };
                            let dist = actual_vec.length();
                            if !exclude {
                                let dir_ok = current_tubing.is_dir_ok();
                                let dist_ok = dist > TUBI_TOL;
                                let same_dir_bad = same_dir;
                                if current_tubing.leave_refno == branch_refno {
                                    debug_model!(
                                        "current_tubing: {:?}, 管道 bran 开头存在直段候选.",
                                        &current_tubing
                                    );
                                    current_tubing.tubi_size = h_tubi_size;
                                } else {
                                    let lstube_cat_ref = aios_core::query_single_by_paths(
                                        current_tubing.leave_refno,
                                        &["->LSTU->CATR"],
                                        &["REFNO"],
                                    )
                                    .await
                                    .map(|x| x.get_refno_or_default())
                                    .unwrap_or_default();
                                    current_tubing.tubi_size = fast_model::query_tubi_size(
                                        current_tubing.leave_refno,
                                        lstube_cat_ref,
                                        is_hang,
                                    )
                                    .await?;
                                }
                                debug_model!(
                                    "current_tubing.tubi_size: {:?}",
                                    &current_tubing.tubi_size
                                );
                                tubi_geo_hash =
                                    if matches!(current_tubing.tubi_size, TubiSize::BoxSize(_)) {
                                        BOXI_GEO_HASH
                                    } else {
                                        TUBI_GEO_HASH
                                    };
                                let transform = (if !dir_ok {
                                    build_tubi_transform_from_segment(
                                        current_tubing.start_pt,
                                        current_tubing.end_pt,
                                        &current_tubing.tubi_size,
                                    )
                                } else {
                                    current_tubing.get_transform()
                                })
                                .or_else(|| {
                                debug_model!(
                                    "[BRAN_TUBI] 直段 {} -> {} 无法计算 transform，使用 fallback",
                                    current_tubing.leave_refno.to_e3d_id(),
                                    current_tubing.arrive_refno.to_e3d_id()
                                );
                                let mut fallback = Transform::IDENTITY;
                                fallback.translation = current_tubing.start_pt;
                                Some(fallback)
                            });
                                if let Some(t) = transform {
                                    if is_debug_branch {
                                        debug_model!(
                                            "[BRAN_TUBI][DBG][segment] leave={} arrive={} idx={} dist={:.3} dir_ok={} same_dir_bad={} start={} end={} dir={}",
                                            current_tubing.leave_refno.to_e3d_id(),
                                            current_tubing.arrive_refno.to_e3d_id(),
                                            current_tubing.index,
                                            dist,
                                            dir_ok,
                                            same_dir_bad,
                                            to_pdms_vec_str(&current_tubing.start_pt, false),
                                            to_pdms_vec_str(&current_tubing.end_pt, false),
                                            to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                                        );
                                        debug_model!(
                                            "[BRAN_TUBI][DBG][segment] trans=({:.3},{:.3},{:.3}) scale=({:.3},{:.3},{:.3}) rot=({:.6},{:.6},{:.6},{:.6})",
                                            t.translation.x, t.translation.y, t.translation.z,
                                            t.scale.x, t.scale.y, t.scale.z,
                                            t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                                        );
                                        if let Ok(mut f) = std::fs::OpenOptions::new()
                                            .create(true)
                                            .append(true)
                                            .open("output/tubi_dbg.txt")
                                        {
                                            let _ = writeln!(
                                                f,
                                                "[segment] leave={} arrive={} idx={} dist={:.3} dir_ok={} same_dir_bad={} start={} end={} dir={} trans=({:.3},{:.3},{:.3}) scale=({:.3},{:.3},{:.3}) rot=({:.6},{:.6},{:.6},{:.6})",
                                                current_tubing.leave_refno.to_e3d_id(),
                                                current_tubing.arrive_refno.to_e3d_id(),
                                                current_tubing.index,
                                                dist,
                                                dir_ok,
                                                same_dir_bad,
                                                to_pdms_vec_str(&current_tubing.start_pt, false),
                                                to_pdms_vec_str(&current_tubing.end_pt, false),
                                                to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                                                t.translation.x, t.translation.y, t.translation.z,
                                                t.scale.x, t.scale.y, t.scale.z,
                                                t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                                            );
                                        }
                                    }
                                    if dist_ok {
                                        let aabb = shared::aabb_apply_transform(&unit_cyli_aabb, &t);
                                        let aabb_hash = gen_aabb_hash(&aabb);
                                        tubi_aabb_map
                                            .entry(aabb_hash.to_string())
                                            .or_insert_with(|| aabb.clone());
                                        let trans_hash = gen_bevy_transform_hash(&t);
                                        tubi_trans_map.entry(trans_hash).or_insert_with(|| {
                                            serde_json::to_string(&t)
                                                .unwrap_or_else(|_| "null".to_string())
                                        });
                                        // 对于 tubing，owner 应该是 BRAN/HANG 本身，而不是 BRAN 的 owner
                                        let owner_refno = branch_refno;
                                        let owner_type = branch_att.get_type_str().to_string();
                                        // 中间直段：axis_map[0]=ARRIVE, axis_map[1]=LEAVE
                                        let arrive_axis_pt = axis_map[0].pt.to_array();
                                        let leave_axis_pt = axis_map[1].pt.to_array();
                                        tubi_shape_insts_data.insert_tubi(
                                            current_tubing.leave_refno,
                                            EleGeosInfo {
                                                refno: current_tubing.leave_refno,
                                                sesno: branch_att.sesno(),
                                                owner_refno,
                                                owner_type,
                                                cata_hash: Some(tubi_geo_hash.to_string()),
                                                visible: true,
                                                generic_type: get_generic_type(
                                                    current_tubing.leave_refno,
                                                )
                                                .await
                                                .unwrap_or_default(),
                                                aabb: Some(aabb),
                                                world_transform: t,
                                                tubi_start_pt: Some(current_tubing.start_pt),
                                                tubi_end_pt: Some(current_tubing.end_pt),
                                                arrive_axis_pt: Some(arrive_axis_pt),
                                                leave_axis_pt: Some(leave_axis_pt),
                                                is_solid: true,
                                                ..Default::default()
                                            },
                                        );
                                        let start_hash = RsVec3(current_tubing.start_pt).gen_hash();
                                        if !tubi_pts_map.contains_key(&start_hash) {
                                            if let Ok(serialized) = serde_json::to_string(&RsVec3(current_tubing.start_pt)) {
                                                tubi_pts_map.insert(start_hash, serialized);
                                            }
                                        }
                                        let end_hash = RsVec3(current_tubing.end_pt).gen_hash();
                                        if !tubi_pts_map.contains_key(&end_hash) {
                                            if let Ok(serialized) = serde_json::to_string(&RsVec3(current_tubing.end_pt)) {
                                                tubi_pts_map.insert(end_hash, serialized);
                                            }
                                        }

                                        let arrive_hash = RsVec3(Vec3::from(arrive_axis_pt)).gen_hash();
                                        if !tubi_pts_map.contains_key(&arrive_hash) {
                                            if let Ok(serialized) =
                                                serde_json::to_string(&RsVec3(Vec3::from(arrive_axis_pt)))
                                            {
                                                tubi_pts_map.insert(arrive_hash, serialized);
                                            }
                                        }
                                        let leave_hash = RsVec3(Vec3::from(leave_axis_pt)).gen_hash();
                                        if !tubi_pts_map.contains_key(&leave_hash) {
                                            if let Ok(serialized) =
                                                serde_json::to_string(&RsVec3(Vec3::from(leave_axis_pt)))
                                            {
                                                tubi_pts_map.insert(leave_hash, serialized);
                                            }
                                        }
                                        debug_model!(
                                            "[BRAN_TUBI] 写入直段 {} -> {}, dist={:.3}, dir_ok={}, same_dir={}",
                                            current_tubing.leave_refno.to_e3d_id(),
                                            current_tubing.arrive_refno.to_e3d_id(),
                                            dist,
                                            dir_ok,
                                            same_dir_bad
                                        );
                                        let sql = format!(
                                            "relate {}->tubi_relate:[{}, {}]->{}  \
                                        set geo=inst_geo:⟨{tubi_geo_hash}⟩,aabb=aabb:⟨{aabb_hash}⟩,world_trans=trans:⟨{trans_hash}⟩, start_pt=vec3:⟨{start_hash}⟩, end_pt=vec3:⟨{end_hash}⟩, arrive_axis=vec3:⟨{arrive_hash}⟩, leave_axis=vec3:⟨{leave_hash}⟩, bore_size={}, bad=false, system={}, dt=fn::ses_date({});",
                                            current_tubing.leave_refno.to_pe_key(),
                                            branch_refno.to_pe_key(),
                                            current_tubing.index,
                                            current_tubing.arrive_refno.to_pe_key(),
                                            current_tubing.tubi_size.to_string(),
                                            owner_refno.to_pe_key(),
                                            current_tubing.leave_refno.to_pe_key(),
                                        );
                                        tubi_relates.push(sql);
                                        current_tubing.index += 1;
                                        tubi_count += 1;
                                    } else {
                                        debug_model!(
                                            "[BRAN_TUBI] 跳过直段（距离过短）: {} -> {}, dist={:.3}, TUBI_TOL={}",
                                            current_tubing.leave_refno.to_e3d_id(),
                                            current_tubing.arrive_refno.to_e3d_id(),
                                            dist,
                                            TUBI_TOL
                                        );
                                    }
                                } else {
                                    debug_model!(
                                        "[BRAN_TUBI] 直段 {} -> {} 无法计算 transform，跳过几何插入",
                                        current_tubing.leave_refno.to_e3d_id(),
                                        current_tubing.arrive_refno.to_e3d_id()
                                    );
                                }
                            }
                        }
                        {
                            let l_dir = axis_map[1].dir.as_ref().map(|x| **x).unwrap_or_default();
                            let ref_dir = axis_map[1]
                                .ref_dir
                                .as_ref()
                                .map(|x| **x)
                                .unwrap_or_default();
                            let mut l_ref_dir = world_trans
                                .to_matrix()
                                .transform_vector3(ref_dir)
                                .normalize_or_zero();
                            if l_ref_dir.dot(l_dir) >= 0.99 {
                                let cond = if l_dir.cross(ref_dir).z >= 0.0 {
                                    1.0
                                } else {
                                    -1.0
                                };
                                l_ref_dir = ref_dir * cond;
                            }
                            if !skip {
                                let l_pos = &axis_map[1].pt;
                                current_tubing.start_pt = **l_pos;
                                current_tubing.desire_leave_dir = l_dir;
                                current_tubing.leave_ref_dir = if l_ref_dir.is_normalized() {
                                    Some(l_ref_dir)
                                } else {
                                    None
                                };
                                current_tubing.leave_refno = refno;
                            }
                        }
                    }
                }

                if index == len - 1 && !exclude {
                    let last_dist = bran_ttube_pt.distance(current_tubing.start_pt);
                    current_tubing.end_pt = bran_ttube_pt;
                    current_tubing.arrive_refno = tref;
                    current_tubing.desire_arrive_dir = tdir;
                    let actual_dir =
                        (current_tubing.end_pt - current_tubing.start_pt).normalize_or_zero();
                    if actual_dir.length_squared() > 0.0
                        && current_tubing.desire_arrive_dir.dot(actual_dir) < 0.99
                    {
                        current_tubing.desire_arrive_dir = actual_dir;
                    }
                    let dir_ok = current_tubing.is_dir_ok();
                    let dist_ok = last_dist > TUBI_TOL;
                    debug_model!(
                        "[BRAN_TUBI] 最后一段候选: leave_refno={}, arrive_refno={}, last_dist={:.3}, dir_ok={}, dist_ok={}",
                        current_tubing.leave_refno.to_e3d_id(),
                        current_tubing.arrive_refno.to_e3d_id(),
                        last_dist,
                        dir_ok,
                        dist_ok
                    );
                    if matches!(current_tubing.tubi_size, TubiSize::None) {
                        let lstube_cat_ref = aios_core::query_single_by_paths(
                            current_tubing.leave_refno,
                            &["->LSTU->CATR"],
                            &["REFNO"],
                        )
                        .await
                        .map(|x| x.get_refno_or_default())
                        .unwrap_or_default();
                        current_tubing.tubi_size = fast_model::query_tubi_size(
                            current_tubing.leave_refno,
                            lstube_cat_ref,
                            is_hang,
                        )
                        .await?;
                    }
                    let transform = (if !dir_ok {
                        build_tubi_transform_from_segment(
                            current_tubing.start_pt,
                            current_tubing.end_pt,
                            &current_tubing.tubi_size,
                        )
                    } else {
                        current_tubing.get_transform()
                    })
                    .or_else(|| {
                        debug_model!(
                            "[BRAN_TUBI] 最后一段 {} -> {} 无法计算 transform，使用 fallback",
                            current_tubing.leave_refno.to_e3d_id(),
                            current_tubing.arrive_refno.to_e3d_id()
                        );
                        let mut fallback = Transform::IDENTITY;
                        fallback.translation = current_tubing.start_pt;
                        Some(fallback)
                    });
                    if let Some(t) = transform {
                        if is_debug_branch {
                            debug_model!(
                                "[BRAN_TUBI][DBG][last] leave={} arrive={} idx={} dist={:.3} dir_ok={} start={} end={} dir={}",
                                current_tubing.leave_refno.to_e3d_id(),
                                current_tubing.arrive_refno.to_e3d_id(),
                                current_tubing.index,
                                last_dist,
                                dir_ok,
                                to_pdms_vec_str(&current_tubing.start_pt, false),
                                to_pdms_vec_str(&current_tubing.end_pt, false),
                                to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                            );
                            debug_model!(
                                "[BRAN_TUBI][DBG][last] trans=({:.3},{:.3},{:.3}) scale=({:.3},{:.3},{:.3}) rot=({:.6},{:.6},{:.6},{:.6})",
                                t.translation.x, t.translation.y, t.translation.z,
                                t.scale.x, t.scale.y, t.scale.z,
                                t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                            );
                            if let Ok(mut f) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open("output/tubi_dbg.txt")
                            {
                                let _ = writeln!(
                                    f,
                                    "[last] leave={} arrive={} idx={} dist={:.3} dir_ok={} start={} end={} dir={} trans=({:.3},{:.3},{:.3}) scale=({:.3},{:.3},{:.3}) rot=({:.6},{:.6},{:.6},{:.6})",
                                    current_tubing.leave_refno.to_e3d_id(),
                                    current_tubing.arrive_refno.to_e3d_id(),
                                    current_tubing.index,
                                    last_dist,
                                    dir_ok,
                                    to_pdms_vec_str(&current_tubing.start_pt, false),
                                    to_pdms_vec_str(&current_tubing.end_pt, false),
                                    to_pdms_vec_str(&current_tubing.desire_arrive_dir, false),
                                    t.translation.x, t.translation.y, t.translation.z,
                                    t.scale.x, t.scale.y, t.scale.z,
                                    t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w,
                                );
                            }
                        }
                        if dist_ok {
                            let aabb = shared::aabb_apply_transform(&unit_cyli_aabb, &t);
                            let aabb_hash = gen_aabb_hash(&aabb);
                            tubi_aabb_map
                                .entry(aabb_hash.to_string())
                                .or_insert_with(|| aabb.clone());
                            let trans_hash = gen_bevy_transform_hash(&t);
                            tubi_trans_map.entry(trans_hash).or_insert_with(|| {
                                serde_json::to_string(&t).unwrap_or_else(|_| "null".to_string())
                            });
                            // 对于 tubing，owner 应该是 BRAN/HANG 本身，而不是 BRAN 的 owner
                            let owner_refno = branch_refno;
                            let owner_type = branch_att.get_type_str().to_string();
                            // 最后一段：leave=上一元件的LEAVE点, arrive=BRAN的TPOS
                            let arrive_axis_pt = bran_ttube_pt.to_array();
                            let leave_axis_pt = current_tubing.start_pt.to_array();
                            tubi_shape_insts_data.insert_tubi(
                                current_tubing.leave_refno,
                                EleGeosInfo {
                                    refno: current_tubing.leave_refno,
                                    sesno: branch_att.sesno(),
                                    owner_refno,
                                    owner_type,
                                    cata_hash: Some(tubi_geo_hash.to_string()),
                                    visible: true,
                                    generic_type: get_generic_type(current_tubing.leave_refno)
                                        .await
                                        .unwrap_or_default(),
                                    aabb: Some(aabb),
                                    world_transform: t,
                                    tubi_start_pt: Some(current_tubing.start_pt),
                                    tubi_end_pt: Some(current_tubing.end_pt),
                                    arrive_axis_pt: Some(arrive_axis_pt),
                                    leave_axis_pt: Some(leave_axis_pt),
                                    is_solid: true,
                                    ..Default::default()
                                },
                            );
                            let start_hash = RsVec3(current_tubing.start_pt).gen_hash();
                            if !tubi_pts_map.contains_key(&start_hash) {
                                if let Ok(serialized) = serde_json::to_string(&RsVec3(current_tubing.start_pt)) {
                                    tubi_pts_map.insert(start_hash, serialized);
                                }
                            }
                            let end_hash = RsVec3(current_tubing.end_pt).gen_hash();
                            if !tubi_pts_map.contains_key(&end_hash) {
                                if let Ok(serialized) = serde_json::to_string(&RsVec3(current_tubing.end_pt)) {
                                    tubi_pts_map.insert(end_hash, serialized);
                                }
                            }
                            let arrive_hash = RsVec3(Vec3::from(arrive_axis_pt)).gen_hash();
                            if !tubi_pts_map.contains_key(&arrive_hash) {
                                if let Ok(serialized) =
                                    serde_json::to_string(&RsVec3(Vec3::from(arrive_axis_pt)))
                                {
                                    tubi_pts_map.insert(arrive_hash, serialized);
                                }
                            }
                            let leave_hash = RsVec3(Vec3::from(leave_axis_pt)).gen_hash();
                            if !tubi_pts_map.contains_key(&leave_hash) {
                                if let Ok(serialized) =
                                    serde_json::to_string(&RsVec3(Vec3::from(leave_axis_pt)))
                                {
                                    tubi_pts_map.insert(leave_hash, serialized);
                                }
                            }
                            tubi_relates.push(format!(
                            "relate {}->tubi_relate:[{}, {}]->{}  \
                            set geo=inst_geo:⟨{tubi_geo_hash}⟩,aabb=aabb:⟨{aabb_hash}⟩,world_trans=trans:⟨{trans_hash}⟩, start_pt=vec3:⟨{start_hash}⟩, end_pt=vec3:⟨{end_hash}⟩, arrive_axis=vec3:⟨{arrive_hash}⟩, leave_axis=vec3:⟨{leave_hash}⟩, bore_size={}, bad=false, system={}, dt=fn::ses_date({});",
                            current_tubing.leave_refno.to_pe_key(),
                            branch_refno.to_pe_key(),
                            current_tubing.index,
                            current_tubing.arrive_refno.to_pe_key(),
                            current_tubing.tubi_size.to_string(),
                            owner_refno.to_pe_key(),
                            current_tubing.leave_refno.to_pe_key(),
                        ));
                            current_tubing.index += 1;
                            tubi_count += 1;
                        } else {
                            debug_model!(
                                "[BRAN_TUBI] 跳过最后一段（距离过短）: last_dist={:.3}, TUBI_TOL={}",
                                last_dist,
                                TUBI_TOL
                            );
                        }
                    }
                }
                leave_type = arrive_type.to_string();
            }

            let branch_tubi_added: i32 = tubi_count - branch_tubi_before;
            debug_model!(
                "[BRAN_TUBI] 分支处理完成: refno={}, 生成 tubi 段数={}",
                branch_refno.to_string(),
                branch_tubi_added
            );

            #[cfg(feature = "profile")]
            {
                let branch_duration = branch_item_start.elapsed();
                tracing::debug!(
                    branch_refno = ?branch_refno,
                    children_count = children.len(),
                    processing_ms = branch_duration.as_micros() as f64 / 1000.0,
                    "BRAN branch item processed"
                );
            }
        }
        process_branch_time = t_process_branch.elapsed().as_millis();

        #[cfg(feature = "profile")]
        tracing::info!(
            branch_count = branch_map.len(),
            tubi_generated = tubi_count,
            total_time_ms = process_branch_time,
            avg_time_per_branch_ms = if branch_map.len() > 0 {
                process_branch_time / branch_map.len() as u128
            } else {
                0
            },
            "BRAN Tubing generation completed"
        );

        // 提取tubi相关的refno列表
        tubi_refnos = tubi_shape_insts_data
            .inst_tubi_map
            .iter()
            .map(|(refno, _)| refno.to_pe_key())
            .collect();

        let t_send_data = Instant::now();
        if tubi_shape_insts_data.inst_cnt() > 0 {
            sender
                .send(tubi_shape_insts_data)
                .expect("send tubi shape_insts_data failed.");
        }
        send_data_time = t_send_data.elapsed().as_millis();

        tubi_query_time = 0;
        if !tubi_relates.is_empty() {
            // 先补齐 tubi_relate 依赖的 trans/aabb/vec3 记录：
            // - RELATE 若先执行，可能会“隐式创建”空记录（d = NONE）
            // - trans/vec3 若随后用 INSERT IGNORE，会因为记录已存在而跳过，导致 d 永远为空
            if !tubi_trans_map.is_empty() {
                if let Err(e) = crate::fast_model::utils::save_transforms_to_surreal(&tubi_trans_map).await {
                    debug_model!("[BRAN_TUBI] 保存 trans 失败: {}", e);
                    panic!("保存 trans 失败: {}", e);
                }
            }
            if !tubi_aabb_map.is_empty() {
                crate::fast_model::utils::save_aabb_to_surreal(&tubi_aabb_map).await;
            }
            if !tubi_pts_map.is_empty() {
                crate::fast_model::utils::save_pts_to_surreal(&tubi_pts_map).await;
            }

            let sql = tubi_relates.join("");
            debug_model!(
                "[BRAN_TUBI] 准备写入 {} 条 tubi_relate 记录，示例 SQL: {}",
                tubi_relates.len(),
                tubi_relates
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("<empty>")
            );

            let t_query = Instant::now();
            if let Err(e) = SUL_DB.query(sql).await {
                debug_model!("[BRAN_TUBI] 写入 tubi_relate 失败: {}", e);
                // 保持原来的 unwrap 语义
                panic!("写入 tubi_relate 失败: {}", e);
            }
            tubi_query_time = t_query.elapsed().as_millis();
            debug_model!(
                "[BRAN_TUBI] 写入 tubi_relate 成功，用时 {} ms",
                tubi_query_time
            );

            // 不再更新PE表的has_tubi字段，直接使用tubi_relate表判断
            debug_model!(
                "[BRAN_TUBI] 跳过更新 has_tubi 标记，改用 tubi_relate 表判断，refnos: {}",
                tubi_refnos.join(",")
            );
        } else {
            debug_model!("[BRAN_TUBI] tubi_relates 为空，本次未写入任何 tubi_relate 记录");
        }
    }

    // 获取并打印汇总统计信息
    let mut time_stats = HashMap::new();
    if let Ok(stats) = Arc::try_unwrap(total_time_stats) {
        time_stats = stats.into_inner();
    }

    // 添加分支处理的时间统计
    if process_branch {
        time_stats.insert("process_branch".to_string(), process_branch_time as u64);
        time_stats.insert("get_children".to_string(), db_time_get_children as u64);
        time_stats.insert("get_branch_att".to_string(), db_time_get_branch_att as u64);
        time_stats.insert(
            "get_branch_transform".to_string(),
            db_time_get_branch_transform as u64,
        );
        time_stats.insert("send_data".to_string(), send_data_time as u64);
        time_stats.insert("tubi_query".to_string(), tubi_query_time as u64);
    }

    // 打印汇总统计信息
    println!("\n==== 数据库操作总耗时统计 (ms) ====");
    let mut stats_vec: Vec<(String, u64)> =
        time_stats.iter().map(|(k, v)| (k.clone(), *v)).collect();
    stats_vec.sort_by(|a, b| b.1.cmp(&a.1)); // 按耗时降序排序

    #[cfg(feature = "profile")]
    {
        for (op_name, time) in stats_vec {
            println!("{}: {} ms", op_name, time);
        }
        let timestamp = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string();
        tracing::info!(
            timestamp = timestamp,
            unique_cata_cnt = unique_cata_cnt,
            total_time_ms = total_t.elapsed().as_millis() as u64,
            "处理元件库几何体完成"
        );
    }

    let total_elapsed_ms = total_t.elapsed().as_millis();
    println!(
        "处理元件库几何体: {} 花费总时间: {} ms",
        unique_cata_cnt, total_elapsed_ms
    );

    let cate_outcome = if process_cata {
        debug_model_debug!(
            "收集到 tubi_info 数量: {}",
            tubi_info_map.len()
        );
        Some(CateGenOutcome {
            local_al_map: local_al_map.clone(),
            tubi_info_map: tubi_info_map.clone(),
            time_stats: time_stats.clone(),
            unique_cata_cnt,
            elapsed_ms: total_elapsed_ms,
        })
    } else {
        None
    };

    let branch_outcome = if process_branch {
        Some(BranchTubiOutcome {
            tubi_relates,
            tubi_refnos,
            time_stats,
            tubi_count,
            elapsed_ms: process_branch_time,
        })
    } else {
        None
    };

    Ok(GenOutcome {
        cate: cate_outcome,
        branch: branch_outcome,
    })
}

//收集ngmr的信息
pub async fn query_ngmr_owner(
    refno: RefnoEnum,
    ngmr_geo_refno: RefnoEnum,
) -> anyhow::Result<Vec<RefnoEnum>> {
    let att = aios_core::get_named_attmap(refno).await.unwrap_or_default();
    let owner = att.get_owner();
    let c_ref = att.get_foreign_refno("CREF");
    let ance_result = aios_core::query_filter_ancestors(refno.clone(), &NGMR_OWN_TYPES).await?;
    let o_ref = ance_result.into_iter().next();
    let geo_att = aios_core::get_named_attmap(ngmr_geo_refno)
        .await
        .unwrap_or_default();
    let removed_type =
        NgmrRemovedType::try_from(geo_att.get_i32("NAPP").unwrap_or(-1)).unwrap_or_default();
    let mut target_refnos = vec![];
    match removed_type {
        NgmrRemovedType::AsDefault => {
            if let Some(o_refno) = o_ref {
                let o_type = aios_core::get_type_name(o_refno).await.unwrap_or_default();
                if CIVIL_TYPES.contains(&o_type.as_str()) {
                    target_refnos.push(o_refno);
                }
            }
        }
        NgmrRemovedType::Nothing => {}
        NgmrRemovedType::Attached => {
            c_ref.map(|x| target_refnos.push(x));
        }
        NgmrRemovedType::Owner => {
            o_ref.map(|x| target_refnos.push(x));
        }
        NgmrRemovedType::Item => target_refnos.push(refno),
        NgmrRemovedType::AttachedAndOwner => {
            c_ref.map(|x| target_refnos.push(x));
            o_ref.map(|x| target_refnos.push(x));
        }
        NgmrRemovedType::AttachedAndItem => {
            c_ref.map(|x| target_refnos.push(x));
            target_refnos.push(refno)
        }
        NgmrRemovedType::OwnerAndItem => {
            o_ref.map(|x| target_refnos.push(x));
            target_refnos.push(refno)
        }
        NgmrRemovedType::All => {
            c_ref.map(|x| target_refnos.push(x));
            o_ref.map(|x| target_refnos.push(x));
            target_refnos.push(refno);
        }
    }
    Ok(target_refnos)
}

// ============================================================================
// 基于 tubi_info 的独立 Tubi 生成（第二阶段）
// ============================================================================

/// 基于数据库 tubi_info 表独立生成 BRAN/HANG tubing
/// 
/// 这是两阶段 BRAN 生成的第二阶段：
/// - 阶段 1: gen_cata_instances() 生成元件几何 + 写入 tubi_info
/// - 阶段 2: gen_tubi_from_db() 读取 tubi_info 生成 tubi_relate
/// 
/// # 参数
/// - `db_option`: 数据库配置
/// - `branch_refnos`: BRAN/HANG 根节点 refno 列表
/// - `sjus_map_arc`: SJUS 调整 map
/// - `sender`: 几何数据发送通道
pub async fn gen_tubi_from_db(
    db_option: Arc<DbOptionExt>,
    branch_refnos: &[RefnoEnum],
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
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
    
    debug_model!("[gen_tubi_from_db] 开始处理 {} 个 BRAN/HANG", branch_refnos.len());
    
    // 1. 从数据库查询 arrive/leave 点（基于 tubi_info）
    let al_map = aios_core::rs_surreal::point::query_arrive_leave_from_tubi_info(branch_refnos).await?;
    
    debug_model!(
        "[gen_tubi_from_db] 从 tubi_info 获取到 {} 个元件的 arrive/leave 点",
        al_map.len()
    );
    
    // 2. 转换为 local_al_map 格式
    let local_al_map: Arc<DashMap<RefnoEnum, [CateAxisParam; 2]>> = Arc::new(al_map);
    
    // 3. 查询 branch 下的子元件
    let branch_map: DashMap<RefnoEnum, Vec<SPdmsElement>> = DashMap::new();
    for &branch_refno in branch_refnos {
        // 约定：BRAN/HANG 的层级/过滤查询统一走 indextree（TreeIndex），避免依赖 SurrealDB 的图查询。
        match crate::fast_model::gen_model::tree_index_manager::TreeIndexManager::collect_children_elements_from_tree(branch_refno).await {
            Ok(children) => {
                if !children.is_empty() {
                    branch_map.insert(branch_refno, children);
                }
            }
            Err(e) => {
                debug_model!(
                    "[gen_tubi_from_db] TreeIndex 查询子元件失败: {} - {}",
                    branch_refno,
                    e
                );
            }
        }
    }
    
    // 4. 调用现有的 gen_branch_tubi 逻辑
    let outcome = gen_branch_tubi(
        db_option,
        Arc::new(branch_map),
        sjus_map_arc,
        sender,
        local_al_map,
    )
    .await?;
    
    let elapsed = start_time.elapsed().as_millis();
    debug_model!(
        "[gen_tubi_from_db] 完成，生成 {} 条 tubi，耗时 {} ms",
        outcome.tubi_count,
        elapsed
    );
    
    Ok(outcome)
}
