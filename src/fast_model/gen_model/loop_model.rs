use crate::consts::*;
use crate::data_interface::interface::PdmsDataInterface;
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::fast_model::gen_model::is_e3d_debug_enabled;
use crate::fast_model::query_provider;
use crate::fast_model::{SEND_INST_SIZE, debug_model_warn, shared};
use crate::options::DbOptionExt;
use aios_core::RefU64;
use aios_core::Transform;
use aios_core::geometry::*;
use aios_core::options::DbOption;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::*;
use aios_core::prim_geo::{Extrusion, Revolution};
use aios_core::shape::pdms_shape::{BrepShapeTrait, VerifiedShape};
use dashmap::DashMap;
use glam::Vec3;
use parry3d::bounding_volume::*;
use parry3d::math::Isometry;
use std::mem::take;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, RwLock};
// LoopInput 已随 model_cache 模块移除

///处理带有loop的元件
pub async fn gen_loop_geos(
    db_option: Arc<DbOptionExt>,
    loop_owner_refnos: &[RefnoEnum],
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<bool> {
    let t = Instant::now();
    let batch_size = db_option.inner.gen_model_batch_size;
    let loop_owner_cnt = loop_owner_refnos.len();
    if loop_owner_cnt == 0 {
        return Ok(true);
    }
    //处理loop elements
    //todo 暂时不用多线程，有一些问题
    let mut batch_chunks_cnt = 16usize.min(loop_owner_cnt.max(1));
    let mut batch_size = (loop_owner_cnt + batch_chunks_cnt - 1) / batch_chunks_cnt;
    if batch_size == 0 {
        batch_size = 1;
    }
    //如果只有一个元件，就不分块了
    if batch_size == 1 {
        batch_chunks_cnt = loop_owner_cnt;
    } else {
        batch_chunks_cnt = (loop_owner_cnt + batch_size - 1) / batch_size;
    }
    let mut handles = vec![];
    // dbg!(&loop_owner_refnos);
    let all_refnos = Arc::new(loop_owner_refnos.to_vec());
    for i in 0..batch_chunks_cnt {
        let all_loop_owner_refnos = all_refnos.clone();
        let sjus_map_clone = sjus_map_arc.clone();
        let sender = sender.clone();
        let db_option = db_option.clone();
        let handle = tokio::spawn(async move {
            let start_idx = i * batch_size;
            if start_idx >= loop_owner_cnt {
                if is_e3d_debug_enabled() {
                    println!(
                        "[gen_loop_geos] 批次 {} 起始索引 {} 超出总长度 {}, 跳过",
                        i, start_idx, loop_owner_cnt
                    );
                }
                return Ok::<_, anyhow::Error>(());
            }
            let mut end_idx = start_idx + batch_size;
            if end_idx > loop_owner_cnt {
                end_idx = loop_owner_cnt;
            }
            if is_e3d_debug_enabled() {
                println!("当前范围: {start_idx} ~ {end_idx}");
            }

            // ── 批量预取：attmap + transform 并发 ──
            {
                let batch_refnos: Vec<RefnoEnum> =
                    all_loop_owner_refnos[start_idx..end_idx].to_vec();
                let attmap_futs: Vec<_> = batch_refnos
                    .iter()
                    .map(|&r| aios_core::get_named_attmap(r))
                    .collect();
                let transform_fut = crate::fast_model::gen_model::transform_cache::get_world_transforms_cache_first_batch(
                    Some(db_option.as_ref()),
                    &batch_refnos,
                );
                let _ = tokio::join!(futures::future::join_all(attmap_futs), transform_fut,);
            }

            let mut shape_insts_data = ShapeInstancesData::default();
            for j in start_idx..end_idx {
                let target_refno = all_loop_owner_refnos[j];
                let mut target_att = aios_core::get_named_attmap(target_refno)
                    .await
                    .unwrap_or_default();
                let target_type = target_att.get_type_str();
                let Ok(Some(mut trans_origin)) =
                    crate::fast_model::gen_model::transform_cache::get_world_transform_cache_first(
                        Some(db_option.as_ref()),
                        target_refno,
                    )
                    .await
                else {
                    continue;
                };
                //判断父节点是否有SJUS，需要调整位置
                #[cfg(feature = "profile")]
                let pane_sjus_start = std::time::Instant::now();

                if (target_type == "FLOOR" || target_type == "PANE" || target_type == "GWALL")
                    && let Some(sjus_adjust) = sjus_map_clone.get(&target_refno)
                {
                    let offset = trans_origin.rotation.mul_vec3(sjus_adjust.value().0);
                    trans_origin.translation += offset;

                    #[cfg(feature = "profile")]
                    tracing::debug!(
                        refno = ?target_refno,
                        noun_type = target_type,
                        sjus_adjust_ms = pane_sjus_start.elapsed().as_micros() as f64 / 1000.0,
                        "PANE/FLOOR/GWALL SJUS adjustment applied"
                    );
                }

                if !target_att.is_neg() {
                    let neg_refnos = query_provider::get_descendants_by_types(
                        target_refno,
                        &GENRAL_NEG_NOUN_NAMES,
                        None,
                    )
                    .await
                    .unwrap_or_default();

                    if !neg_refnos.is_empty() {
                        println!(
                            "🔍 [LOOP] 找到负实体: target={}, neg_count={}",
                            target_refno,
                            neg_refnos.len()
                        );
                    }

                    shape_insts_data.insert_negs(target_refno, &neg_refnos);
                    //检查是否有CMPF
                    let cmpf_refnos =
                        query_provider::get_descendants_by_types(target_refno, &["CMPF"], None)
                            .await
                            .unwrap_or_default();
                    if !cmpf_refnos.is_empty() {
                        //查询cmpf里面的元素
                        let cmpf_neg_refnos = query_provider::query_multi_descendants(
                            &cmpf_refnos,
                            &GENRAL_NEG_NOUN_NAMES,
                        )
                        .await
                        .unwrap_or_default();
                        // dbg!(&cmpf_neg_refnos);
                        shape_insts_data.insert_negs(
                            target_refno,
                            &cmpf_neg_refnos.into_iter().map(|x| x).collect::<Vec<_>>(),
                        );
                    }
                }
                let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&target_att).await;
                let mut geos_info = EleGeosInfo {
                    refno: target_refno,
                    sesno: target_att.sesno(),
                    owner_refno,
                    owner_type,
                    cata_hash: None,
                    visible: true,
                    world_transform: trans_origin,
                    aabb: None,
                    flow_pt_indexs: vec![],
                    ..Default::default()
                };
                let mut geo_hash = 0;
                let mut item_trans = Transform::IDENTITY;
                let mut geo_param = PdmsGeoParam::Unknown;
                let loop_res = match aios_core::fetch_loops_and_height(target_refno).await {
                    Ok(res) => res,
                    Err(e) => {
                        debug_model_warn!(
                            "{}: fetch_loops_and_height 失败: {} (type={})",
                            target_refno,
                            e,
                            target_type
                        );
                        continue;
                    }
                };
                let verts = loop_res.loops;
                let height = loop_res.height;
                debug_model_warn!(
                    "{}: fetch_loops_and_height 结果: loops={}, height={} (type={})",
                    target_refno,
                    verts.len(),
                    height,
                    target_type
                );
                match target_type {
                    "NREV" | "REVO" => {
                        let angle = target_att.get_f32("ANGL").unwrap_or_default();
                        if angle.abs() >= f32::EPSILON {
                            let revo = Box::new(Revolution {
                                verts,
                                angle,
                                ..Default::default()
                            });
                            if revo.check_valid() {
                                // dbg!(&revo);
                                item_trans = revo.get_trans();
                                geo_param =
                                    revo.convert_to_geo_param().unwrap_or(PdmsGeoParam::Unknown);
                                geo_hash = revo.hash_unit_mesh_params();
                            }
                        }
                    }
                    //todo 关于justline，可能需要jusline的信息才能判断中心点
                    "AEXTR" | "NXTR" | "EXTR" | "PANE" | "FLOOR" | "SCREED" | "GWALL" => {
                        #[cfg(feature = "profile")]
                        let extr_start = std::time::Instant::now();

                        if height < f32::EPSILON {
                            debug_model_warn!("{}： 的height太小为: {}", target_refno, height);
                            continue;
                        }
                        // if loop_attr.get_type_str() == "NXTR" {
                        //     if let Some(parent_inst) =
                        //         shape_insts_data.get_inst_info(loop_attr.get_owner())
                        //     {
                        //         if let Some(h) =
                        //             parent_inst.aabb.map(|x| x.bounding_sphere().radius * 2.0)
                        //         {
                        //             height = height.min(h);
                        //             // dbg!(height);
                        //             println!("Height 太长，裁剪为: {}", height);
                        //         }
                        //     }
                        // };
                        //如果有多个loop，都放到 verts 里好了
                        let extrusion = Box::new(Extrusion {
                            verts,
                            height,
                            ..Default::default()
                        });
                        geo_param = extrusion
                            .convert_to_geo_param()
                            .unwrap_or(PdmsGeoParam::Unknown);
                        item_trans = extrusion.get_trans();
                        geo_hash = extrusion.hash_unit_mesh_params();

                        #[cfg(feature = "profile")]
                        {
                            let is_pane_type = matches!(target_type, "PANE" | "FLOOR" | "GWALL");
                            if is_pane_type {
                                tracing::debug!(
                                    refno = ?target_refno,
                                    noun_type = target_type,
                                    height = height,
                                    vert_count = extrusion.verts.len(),
                                    processing_ms = extr_start.elapsed().as_micros() as f64 / 1000.0,
                                    "PANE/FLOOR/GWALL extrusion processed"
                                );
                            }
                        }
                    }
                    _ => {}
                }
                let visible = target_att.is_visible_by_level(None).unwrap_or(true);
                geos_info.visible = visible;
                debug_model_warn!(
                    "{}: geo_hash={}, geo_param={:?}, trans_nan=({},{},{})",
                    target_refno,
                    geo_hash,
                    std::mem::discriminant(&geo_param),
                    item_trans.translation.is_nan(),
                    item_trans.rotation.is_nan(),
                    item_trans.scale.is_nan()
                );
                if item_trans.translation.is_nan()
                    || item_trans.rotation.is_nan()
                    || item_trans.scale.is_nan()
                {
                    debug_model_warn!("{}: 跳过 — Transform 包含 NaN", target_refno);
                    continue;
                }
                let mut tr: Transform = item_trans;
                let unit_flag = geo_param.is_reuse_unit();
                // unit_flag=true 时，落库/入缓存的 geo_param 必须是"单位参数"，
                // 否则同一 geo_hash 复用会被某个实例的绝对尺寸污染，且在保留 scale 时会重复缩放。
                if unit_flag {
                    geo_param = geo_param.to_unit_param();
                }
                // 统一处理 transform.scale 清零逻辑
                crate::fast_model::reuse_unit::normalize_transform_scale(
                    &mut tr, unit_flag, geo_hash,
                );
                //需要判断多个PLOO、LOOP的情况，第二个开始都是负实体
                let geo_type = if target_att.is_neg() {
                    GeoBasicType::Neg
                } else {
                    GeoBasicType::Pos
                };
                let geom_inst = EleInstGeo {
                    geo_hash,
                    refno: target_refno,
                    pts: Default::default(),
                    aabb: None,
                    geo_transform: tr,
                    visible,
                    is_tubi: false,
                    geo_param: geo_param.clone(),
                    geo_type,
                    cata_neg_refnos: Default::default(),
                };
                geos_info.is_solid = geom_inst.geo_type == GeoBasicType::Pos;
                let inst_key = geos_info.get_inst_key();
                shape_insts_data.insert_geos_data(
                    inst_key.clone(),
                    EleInstGeosData {
                        inst_key,
                        refno: target_refno,
                        insts: vec![geom_inst.clone()],
                        aabb: None,
                        type_name: target_att.get_type_str().to_string(),
                    },
                );
                shape_insts_data.insert_info(target_refno, geos_info);

                if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                    sender
                        .send_async(std::mem::take(&mut shape_insts_data))
                        .await
                        .map_err(|_| anyhow::anyhow!("send loop shape_insts_data error"))?;
                    // dbg!("Send loop insts data");
                }
            }

            if shape_insts_data.inst_cnt() > 0 {
                sender
                    .send_async(shape_insts_data)
                    .await
                    .map_err(|_| anyhow::anyhow!("send loop shape_insts_data error"))?;
                // dbg!("Send last loop insts data");
            }
            Ok::<_, anyhow::Error>(())
        });

        handles.push(handle);
    }
    futures::future::join_all(take(&mut handles)).await;
    if is_e3d_debug_enabled() {
        println!(
            "处理loops几何体: {} 花费时间: {} ms",
            loop_owner_cnt,
            t.elapsed().as_millis()
        );
    }
    Ok(true)
}

// [foyer-removal] cache-only 函数已禁用，LoopInput 类型已随 model_cache 移除
/*
pub async fn gen_loop_geos_from_inputs(
    db_option: Arc<DbOptionExt>,
    loop_inputs: std::collections::HashMap<RefnoEnum, LoopInput>,
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<bool> {
    let t = Instant::now();
    let loop_owner_cnt = loop_inputs.len();
    if loop_owner_cnt == 0 {
        return Ok(true);
    }

    // 复用原先的“分块并发”形态；区别在于：所有输入都来自 cache。
    let mut batch_chunks_cnt = 16usize.min(loop_owner_cnt.max(1));
    let mut batch_size = (loop_owner_cnt + batch_chunks_cnt - 1) / batch_chunks_cnt;
    if batch_size == 0 {
        batch_size = 1;
    }
    if batch_size == 1 {
        batch_chunks_cnt = loop_owner_cnt;
    }

    let all_inputs: Arc<Vec<LoopInput>> = Arc::new(loop_inputs.into_values().collect());

    let mut handles = Vec::new();
    for i in 0..batch_chunks_cnt {
        let all_inputs = all_inputs.clone();
        let sjus_map_clone = sjus_map_arc.clone();
        let sender = sender.clone();
        let _db_option = db_option.clone();

        let handle = tokio::spawn(async move {
            let mut shape_insts_data = ShapeInstancesData::default();

            let start_idx = i * batch_size;
            if start_idx >= all_inputs.len() {
                return Ok::<_, anyhow::Error>(());
            }
            let end_idx = (start_idx + batch_size).min(all_inputs.len());

            for j in start_idx..end_idx {
                let input = &all_inputs[j];
                let target_refno = input.refno;
                let target_att = &input.attmap;
                let target_type = target_att.get_type_str();
                if target_type.is_empty() {
                    // 缓存缺少 TYPE 时，避免生成 geo_hash=0 的脏数据。
                    continue;
                }

                // world_transform：以 cache 为准，并按 SJUS 做兼容调整（与旧路径保持一致）。
                let mut trans_origin = input.world_transform;
                if (target_type == "FLOOR" || target_type == "PANE" || target_type == "GWALL")
                    && let Some(sjus_adjust) = sjus_map_clone.get(&target_refno)
                {
                    let offset = trans_origin.rotation.mul_vec3(sjus_adjust.value().0);
                    trans_origin.translation += offset;
                }

                // 负实体：直接使用 cache 预取结果（不再回查 TreeIndex/DB）。
                if !target_att.is_neg() {
                    if !input.neg_refnos.is_empty() {
                        shape_insts_data.insert_negs(target_refno, &input.neg_refnos);
                    }
                    if !input.cmpf_neg_refnos.is_empty() {
                        shape_insts_data.insert_negs(target_refno, &input.cmpf_neg_refnos);
                    }
                }

                let mut geos_info = EleGeosInfo {
                    refno: target_refno,
                    sesno: target_att.sesno(),
                    owner_refno: input.owner_refno,
                    owner_type: input.owner_type.clone(),
                    cata_hash: None,
                    visible: input.visible,
                    world_transform: trans_origin,
                    aabb: None,
                    flow_pt_indexs: vec![],
                    ..Default::default()
                };

                let mut geo_hash = 0;
                let mut item_trans = Transform::IDENTITY;
                let mut geo_param = PdmsGeoParam::Unknown;

                let verts = input.loops.clone();
                let height = input.height;

                match target_type {
                    "NREV" | "REVO" => {
                        let angle = target_att.get_f32("ANGL").unwrap_or_default();
                        if angle.abs() >= f32::EPSILON {
                            let revo = Box::new(Revolution {
                                verts,
                                angle,
                                ..Default::default()
                            });
                            if revo.check_valid() {
                                item_trans = revo.get_trans();
                                geo_param =
                                    revo.convert_to_geo_param().unwrap_or(PdmsGeoParam::Unknown);
                                geo_hash = revo.hash_unit_mesh_params();
                            }
                        }
                    }
                    "AEXTR" | "NXTR" | "EXTR" | "PANE" | "FLOOR" | "SCREED" | "GWALL" => {
                        if height < f32::EPSILON {
                            debug_model_warn!("{}： 的height太小为: {}", target_refno, height);
                            continue;
                        }
                        let extrusion = Box::new(Extrusion {
                            verts,
                            height,
                            ..Default::default()
                        });
                        geo_param = extrusion
                            .convert_to_geo_param()
                            .unwrap_or(PdmsGeoParam::Unknown);
                        item_trans = extrusion.get_trans();
                        geo_hash = extrusion.hash_unit_mesh_params();
                    }
                    _ => {}
                }

                if item_trans.translation.is_nan()
                    || item_trans.rotation.is_nan()
                    || item_trans.scale.is_nan()
                {
                    continue;
                }

                geos_info.visible = input.visible;

                let mut tr: Transform = item_trans;
                let unit_flag = geo_param.is_reuse_unit();
                if unit_flag {
                    geo_param = geo_param.to_unit_param();
                }
                crate::fast_model::reuse_unit::normalize_transform_scale(&mut tr, unit_flag, geo_hash);

                let geo_type = if target_att.is_neg() {
                    GeoBasicType::Neg
                } else {
                    GeoBasicType::Pos
                };

                let geom_inst = EleInstGeo {
                    geo_hash,
                    refno: target_refno,
                    pts: Default::default(),
                    aabb: None,
                    geo_transform: tr,
                    visible: input.visible,
                    is_tubi: false,
                    geo_param: geo_param.clone(),
                    geo_type,
                    cata_neg_refnos: Default::default(),
                };

                geos_info.is_solid = geom_inst.geo_type == GeoBasicType::Pos;
                let inst_key = geos_info.get_inst_key();
                shape_insts_data.insert_geos_data(
                    inst_key.clone(),
                    EleInstGeosData {
                        inst_key,
                        refno: target_refno,
                        insts: vec![geom_inst.clone()],
                        aabb: None,
                        type_name: target_att.get_type_str().to_string(),
                    },
                );
                shape_insts_data.insert_info(target_refno, geos_info);

                if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                    sender
                        .send(std::mem::take(&mut shape_insts_data))
                        .expect("send loop shape_insts_data error");
                }
            }

            if shape_insts_data.inst_cnt() > 0 {
                sender
                    .send(shape_insts_data)
                    .expect("send last loop shape_insts_data error");
            }

            Ok::<_, anyhow::Error>(())
        });

        handles.push(handle);
    }

    futures::future::join_all(take(&mut handles)).await;
    if is_e3d_debug_enabled() {
        println!(
            "[gen_loop_geos_from_inputs] 处理loops几何体: {} 花费时间: {} ms",
            loop_owner_cnt,
            t.elapsed().as_millis()
        );
    }
    Ok(true)
}
*/
