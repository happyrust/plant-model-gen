use crate::fast_model::gen_model::is_e3d_debug_enabled;
use crate::fast_model::gen_model::neg_query;
use crate::fast_model::query_compat::query_filter_deep_children_atts;
use crate::fast_model::{SEND_INST_SIZE, shared};
use crate::options::DbOptionExt;
use crate::{consts::*, e3d_dbg};
use aios_core::Transform;
use aios_core::geometry::*;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::*;
use aios_core::prim_geo::polyhedron::Polygon;
use aios_core::prim_geo::*;
use aios_core::shape::pdms_shape::BrepShapeTrait;
use glam::Vec3;
use std::collections::HashMap;
use std::mem::take;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// 公共工具函数
// ---------------------------------------------------------------------------

/// 计算并发分块参数：返回 (batch_count, batch_size)
fn calculate_batch_chunks(total: usize) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let mut batch_count = 8usize.min(total);
    let mut batch_size = (total + batch_count - 1) / batch_count;
    if batch_size == 0 {
        batch_size = 1;
    }
    if batch_size == 1 {
        batch_count = total;
    } else {
        batch_count = (total + batch_size - 1) / batch_size;
    }
    (batch_count, batch_size)
}

/// 从 CSG shape 构建 EleInstGeo（两个入口函数的核心公共逻辑）。
///
/// 返回 `Some((inst_geo, geo_insts_has_pos))` 或 `None`（表示跳过）。
fn build_inst_geo_from_shape(
    csg_shape: Box<dyn BrepShapeTrait>,
    refno: RefnoEnum,
    visible: bool,
    is_neg: bool,
) -> Option<EleInstGeo> {
    if !csg_shape.check_valid() {
        return None;
    }

    let mut transform = csg_shape.get_trans();
    if transform.translation.is_nan() || transform.rotation.is_nan() || transform.scale.is_nan() {
        return None;
    }

    let mut geo_param = csg_shape
        .convert_to_geo_param()
        .unwrap_or(PdmsGeoParam::Unknown);
    let geo_hash = csg_shape.hash_unit_mesh_params();
    let unit_flag = csg_shape.is_reuse_unit();

    if unit_flag {
        geo_param = csg_shape
            .gen_unit_shape()
            .convert_to_geo_param()
            .unwrap_or(geo_param);
    }

    crate::fast_model::reuse_unit::normalize_transform_scale(&mut transform, unit_flag, geo_hash);

    Some(EleInstGeo {
        geo_hash,
        refno,
        pts: Default::default(),
        aabb: None,
        geo_transform: transform,
        geo_param,
        visible,
        is_tubi: false,
        geo_type: if is_neg {
            GeoBasicType::Neg
        } else {
            GeoBasicType::Pos
        },
        cata_neg_refnos: vec![],
    })
}

/// 从 DB 查询构建多面体 CSG shape（POHE/POLYHE）。
async fn build_polyhedron_from_db(refno: RefnoEnum) -> Option<Box<dyn BrepShapeTrait>> {
    let pgo_refnos = crate::fast_model::query_provider::get_children(refno)
        .await
        .unwrap_or_default();
    if pgo_refnos.is_empty() {
        return None;
    }

    let first_type = aios_core::get_type_name(pgo_refnos[0])
        .await
        .unwrap_or_default();

    let mut polygons = vec![];
    let mut is_polyhe = false;

    if first_type == "POLPTL" {
        is_polyhe = true;
        let mut verts_map = HashMap::new();
        let v_att = crate::fast_model::query_provider::query_multi_descendants_with_self(
            &[pgo_refnos[0]],
            &["POIN"],
            false,
        )
        .await
        .unwrap_or_default();
        for v in v_att.into_iter() {
            let v_attmap = aios_core::get_named_attmap(v).await.unwrap_or_default();
            let pos = v_attmap.get_position().unwrap_or_default();
            verts_map.insert(v, pos);
        }
        let index_loops = query_filter_deep_children_atts(refno, &["LOOPTS"])
            .await
            .unwrap_or_default();
        let index_map = index_loops.iter().fold(HashMap::new(), |mut map, x| {
            let owner = x.get_owner();
            let vx_refnos = x.get_refno_vec("VXREF").unwrap_or_default();
            map.entry(owner).or_insert_with(Vec::new).extend(vx_refnos);
            map
        });
        let loop_atts = query_filter_deep_children_atts(refno, &["POLOOP"])
            .await
            .unwrap_or_default();
        let loops_map = loop_atts.iter().fold(HashMap::new(), |mut map, x| {
            let owner = x.get_owner();
            if let Some(index_refnos) = index_map.get(&x.get_refno_or_default()) {
                map.entry(owner).or_insert_with(Vec::new).push(index_refnos);
            }
            map
        });
        for (_, v) in loops_map {
            let mut loops = vec![];
            for l in v {
                let mut verts: Vec<Vec3> = vec![];
                for index_refno in l {
                    if let Some(vert) = verts_map.get(index_refno) {
                        verts.push(*vert);
                    }
                }
                loops.push(verts);
            }
            polygons.push(Polygon { loops });
        }
    } else {
        for pgo_refno in pgo_refnos {
            let mut verts = vec![];
            let v_att = aios_core::collect_children_filter_attrs(pgo_refno, &[])
                .await
                .unwrap_or_default();
            for v in v_att {
                verts.push(v.get_position().unwrap_or_default());
            }
            polygons.push(Polygon { loops: vec![verts] });
        }
    }

    let shape: Box<dyn BrepShapeTrait> = Box::new(Polyhedron {
        polygons,
        mesh: None,
        is_polyhe,
    });
    Some(shape)
}

/// 从缓存的 PrimPolyExtra 构建多面体 CSG shape。
fn build_polyhedron_from_cache(
    extra: &crate::fast_model::model_cache::geom_input_cache::PrimPolyExtra,
) -> Box<dyn BrepShapeTrait> {
    let polygons = extra
        .polygons
        .iter()
        .map(|p| Polygon {
            loops: p.loops.clone(),
        })
        .collect::<Vec<_>>();
    Box::new(Polyhedron {
        polygons,
        mesh: None,
        is_polyhe: extra.is_polyhe,
    })
}

/// 将已构建的 inst_geo 插入 shape_insts_data，并处理负实体关系。
fn insert_prim_result(
    shape_insts_data: &mut ShapeInstancesData,
    geos_info: EleGeosInfo,
    inst_geo: EleInstGeo,
    neg_refnos: &[RefnoEnum],
    type_name: &str,
) {
    let refno = geos_info.refno;
    let is_solid = inst_geo.geo_type == GeoBasicType::Pos;
    let mut geos_info = geos_info;
    geos_info.is_solid = is_solid;

    if !neg_refnos.is_empty() {
        shape_insts_data.insert_negs(refno, neg_refnos);
    }

    let inst_key = geos_info.get_inst_key();
    shape_insts_data.insert_geos_data(
        inst_key.clone(),
        EleInstGeosData {
            inst_key,
            refno,
            insts: vec![inst_geo],
            aabb: None,
            type_name: type_name.to_string(),
        },
    );
    shape_insts_data.insert_info(refno, geos_info);
}

/// 如果 batch 达到阈值则发送，返回是否发送成功。
fn flush_if_needed(
    shape_insts_data: &mut ShapeInstancesData,
    sender: &flume::Sender<ShapeInstancesData>,
    batch_idx: usize,
    sent_count: &mut usize,
) -> anyhow::Result<()> {
    if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
        e3d_dbg!(
            "[gen_prim_geos] 批次 {} 发送中间数据: {} 个实例",
            batch_idx,
            shape_insts_data.inst_cnt()
        );
        sender
            .send(std::mem::take(shape_insts_data))
            .map_err(|e| anyhow::anyhow!("send prim shape_insts_data error: {}", e))?;
        *sent_count += 1;
    }
    Ok(())
}

/// 发送剩余数据。
fn flush_remaining(
    shape_insts_data: ShapeInstancesData,
    sender: &flume::Sender<ShapeInstancesData>,
    batch_idx: usize,
    sent_count: &mut usize,
) -> anyhow::Result<()> {
    if shape_insts_data.inst_cnt() > 0 {
        e3d_dbg!(
            "[gen_prim_geos] 批次 {} 发送最后数据: {} 个实例",
            batch_idx,
            shape_insts_data.inst_cnt()
        );
        sender
            .send(shape_insts_data)
            .map_err(|e| anyhow::anyhow!("send last prim shape_insts_data error: {}", e))?;
        *sent_count += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 公开入口函数
// ---------------------------------------------------------------------------

/// 生成基本体的几何数据（从 SurrealDB 查询属性）
pub async fn gen_prim_geos(
    db_option: Arc<DbOptionExt>,
    prim_refnos: &[RefnoEnum],
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<bool> {
    let t = Instant::now();
    let prim_cnt = prim_refnos.len();

    e3d_dbg!(
        "[gen_prim_geos] 开始生成基本体几何数据, 总数量: {}",
        prim_cnt
    );

    if prim_cnt == 0 {
        return Ok(true);
    }

    let (batch_chunks_cnt, batch_size) = calculate_batch_chunks(prim_cnt);
    e3d_dbg!(
        "[gen_prim_geos] 分块策略: {} 个批次, 每批 {} 个元素",
        batch_chunks_cnt,
        batch_size
    );

    let all_refnos = Arc::new(prim_refnos.to_vec());
    let processed_cnt = Arc::new(Mutex::new(prim_cnt));
    let mut handles = vec![];

    for i in 0..batch_chunks_cnt {
        let all_refnos = all_refnos.clone();
        let processed_cnt = processed_cnt.clone();
        let sender = sender.clone();
        let db_option = db_option.clone();

        let handle = tokio::spawn(async move {
            let batch_start_time = Instant::now();
            let mut shape_insts_data = ShapeInstancesData::default();
            let start_idx = i * batch_size;
            if start_idx >= prim_cnt {
                return Ok::<_, anyhow::Error>(());
            }
            let end_idx = (start_idx + batch_size).min(prim_cnt);
            let batch_item_count = end_idx - start_idx;

            e3d_dbg!(
                "[gen_prim_geos] 批次 {} 开始: 索引范围 {} ~ {}, 共 {} 个元素",
                i,
                start_idx,
                end_idx,
                batch_item_count
            );

            // ── 批量预取：attmap + transform 并发，neg 走 TreeIndex ──
            let batch_refnos: Vec<RefnoEnum> = all_refnos[start_idx..end_idx].to_vec();
            {
                let t_prefetch = Instant::now();
                let attmap_futs: Vec<_> = batch_refnos
                    .iter()
                    .map(|&r| aios_core::get_named_attmap(r))
                    .collect();
                let transform_fut = crate::fast_model::gen_model::transform_cache::get_world_transforms_cache_first_batch(
                    Some(db_option.as_ref()),
                    &batch_refnos,
                );
                let _ = tokio::join!(futures::future::join_all(attmap_futs), transform_fut,);
                e3d_dbg!(
                    "[gen_prim_geos] 批次 {} 预取 attmap+transform 完成: {} 个, 耗时 {} ms",
                    i,
                    batch_item_count,
                    t_prefetch.elapsed().as_millis()
                );
            }

            let neg_map = {
                let tree_dir = db_option.get_project_output_dir().join("scene_tree");
                neg_query::query_descendants_map_by_dbnum(
                    &tree_dir,
                    &batch_refnos,
                    &GENRAL_NEG_NOUN_NAMES,
                    false,
                )
                .unwrap_or_default()
            };

            // ── 主循环：从缓存读取 ──
            let mut processed_in_batch = 0usize;
            let mut skipped_in_batch = 0usize;
            let mut sent_count = 0usize;

            for j in start_idx..end_idx {
                let refno = all_refnos[j];
                {
                    let mut cnt = processed_cnt.lock().await;
                    *cnt -= 1;
                }

                let trans_result =
                    crate::fast_model::gen_model::transform_cache::get_world_transform_cache_first(
                        Some(db_option.as_ref()),
                        refno,
                    )
                    .await;
                let Ok(Some(trans_origin)) = trans_result else {
                    skipped_in_batch += 1;
                    if let Err(e) = &trans_result {
                        e3d_dbg!(
                            "批次 {} 跳过 refno={}: 获取世界变换失败 - {:?}",
                            i,
                            refno,
                            e
                        );
                    }
                    continue;
                };

                let attr = aios_core::get_named_attmap(refno).await.unwrap_or_default();
                let visible = attr.is_visible_by_level(None).unwrap_or(true);
                let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&attr).await;
                let cur_type = attr.get_type_str();

                let geos_info = EleGeosInfo {
                    refno,
                    sesno: attr.sesno(),
                    owner_refno,
                    owner_type,
                    visible,
                    aabb: None,
                    world_transform: trans_origin,
                    ..Default::default()
                };

                let neg_limit_size: Option<f32> = if GENRAL_NEG_NOUN_NAMES.contains(&cur_type) {
                    Some(1000_000.0)
                } else {
                    None
                };

                let csg_shape = if cur_type == "POHE" || cur_type == "POLYHE" {
                    build_polyhedron_from_db(refno).await
                } else {
                    attr.create_csg_shape(neg_limit_size)
                };

                let Some(csg_shape) = csg_shape else {
                    skipped_in_batch += 1;
                    continue;
                };

                let Some(inst_geo) =
                    build_inst_geo_from_shape(csg_shape, refno, visible, attr.is_neg())
                else {
                    skipped_in_batch += 1;
                    continue;
                };

                let neg_refnos = neg_map.get(&refno).cloned().unwrap_or_default();

                insert_prim_result(
                    &mut shape_insts_data,
                    geos_info,
                    inst_geo,
                    &neg_refnos,
                    cur_type,
                );
                processed_in_batch += 1;

                flush_if_needed(&mut shape_insts_data, &sender, i, &mut sent_count)?;
            }

            flush_remaining(shape_insts_data, &sender, i, &mut sent_count)?;

            e3d_dbg!(
                "[gen_prim_geos] 批次 {} 完成: 处理 {}/{} 个, 跳过 {} 个, 发送 {} 次, 耗时 {} ms",
                i,
                processed_in_batch,
                batch_item_count,
                skipped_in_batch,
                sent_count,
                batch_start_time.elapsed().as_millis()
            );

            Ok::<_, anyhow::Error>(())
        });

        handles.push(handle);
    }

    e3d_dbg!(
        "[gen_prim_geos] 等待所有 {} 个批次任务完成...",
        handles.len()
    );
    let results = futures::future::join_all(take(&mut handles)).await;

    let mut success_count = 0;
    let mut error_count = 0;
    for (idx, result) in results.iter().enumerate() {
        match result {
            Ok(Ok(_)) => success_count += 1,
            Ok(Err(e)) => {
                error_count += 1;
                e3d_dbg!("[gen_prim_geos] 批次 {} 执行错误: {:?}", idx, e);
            }
            Err(e) => {
                error_count += 1;
                e3d_dbg!("[gen_prim_geos] 批次 {} 任务失败: {:?}", idx, e);
            }
        }
    }

    let total_elapsed = t.elapsed();
    e3d_dbg!(
        "[gen_prim_geos] 完成! 总数: {}, 成功批次: {}, 失败批次: {}, 总耗时: {} ms",
        prim_cnt,
        success_count,
        error_count,
        total_elapsed.as_millis()
    );

    if is_e3d_debug_enabled() {
        println!(
            "处理常规基本几何体: {} 花费时间: {} ms",
            prim_cnt,
            total_elapsed.as_millis()
        );
    }
    Ok(true)
}

// [foyer-removal] cache-only 函数已禁用，PrimInput 类型已随 model_cache 移除
/*
pub async fn gen_prim_geos_from_inputs(
    db_option: Arc<DbOptionExt>,
    prim_inputs: HashMap<RefnoEnum, PrimInput>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<bool> {
    let t = Instant::now();
    let batch_size_cfg = db_option.inner.gen_model_batch_size;
    let diag_enabled = std::env::var("GEN_MODEL_DIAG")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    let prim_cnt = prim_inputs.len();
    if prim_cnt == 0 {
        return Ok(true);
    }

    let (batch_chunks_cnt, batch_size) = calculate_batch_chunks(prim_cnt);
    let all_inputs: Arc<Vec<PrimInput>> = Arc::new(prim_inputs.into_values().collect());

    let mut handles = vec![];
    for i in 0..batch_chunks_cnt {
        let all_inputs = all_inputs.clone();
        let sender = sender.clone();
        let diag_enabled = diag_enabled;

        let handle = tokio::spawn(async move {
            let batch_start_time = Instant::now();
            let mut shape_insts_data = ShapeInstancesData::default();
            let start_idx = i * batch_size;
            if start_idx >= all_inputs.len() {
                return Ok::<_, anyhow::Error>(());
            }
            let end_idx = (start_idx + batch_size).min(all_inputs.len());
            if diag_enabled {
                let first = all_inputs[start_idx].refno;
                let last = all_inputs[end_idx - 1].refno;
                println!(
                    "[gen_prim_geos_from_inputs][diag] 批次 {} 开始: range=({}~{}), first={}, last={}, count={}",
                    i,
                    start_idx + 1,
                    end_idx,
                    first,
                    last,
                    end_idx - start_idx
                );
            }

            let mut skipped_in_batch = 0usize;
            let mut processed_in_batch = 0usize;
            let mut sent_count = 0usize;

            for j in start_idx..end_idx {
                let input = &all_inputs[j];
                let refno = input.refno;
                let attr = &input.attmap;
                let visible = input.visible;
                let cur_type = attr.get_type_str();

                if cur_type.is_empty() {
                    skipped_in_batch += 1;
                    continue;
                }

                let geos_info = EleGeosInfo {
                    refno,
                    sesno: attr.sesno(),
                    owner_refno: input.owner_refno,
                    owner_type: input.owner_type.clone(),
                    visible,
                    aabb: None,
                    world_transform: input.world_transform,
                    ..Default::default()
                };

                // 构建 CSG shape
                let neg_limit_size: Option<f32> = if GENRAL_NEG_NOUN_NAMES.contains(&cur_type) {
                    Some(1000_000.0)
                } else {
                    None
                };

                let csg_shape: Option<Box<dyn BrepShapeTrait>> =
                    if cur_type == "POHE" || cur_type == "POLYHE" {
                        input.poly_extra.as_ref().map(build_polyhedron_from_cache)
                    } else {
                        attr.create_csg_shape(neg_limit_size)
                    };

                let Some(csg_shape) = csg_shape else {
                    skipped_in_batch += 1;
                    continue;
                };

                // 构建 inst_geo（复用公共逻辑）
                let Some(inst_geo) = build_inst_geo_from_shape(
                    csg_shape, refno, visible, attr.is_neg(),
                ) else {
                    skipped_in_batch += 1;
                    continue;
                };

                // 插入结果
                insert_prim_result(
                    &mut shape_insts_data,
                    geos_info,
                    inst_geo,
                    &input.neg_refnos,
                    cur_type,
                );
                processed_in_batch += 1;

                flush_if_needed(&mut shape_insts_data, &sender, i, &mut sent_count)?;
            }

            flush_remaining(shape_insts_data, &sender, i, &mut sent_count)?;

            e3d_dbg!(
                "[gen_prim_geos_from_inputs] 批次 {} 完成: processed={}, skipped={}, sent={}, elapsed={} ms (cfg_batch_size={})",
                i, processed_in_batch, skipped_in_batch, sent_count,
                batch_start_time.elapsed().as_millis(), batch_size_cfg
            );
            if diag_enabled {
                println!(
                    "[gen_prim_geos_from_inputs][diag] 批次 {} 完成: processed={}, skipped={}, sent={}, elapsed={} ms",
                    i,
                    processed_in_batch,
                    skipped_in_batch,
                    sent_count,
                    batch_start_time.elapsed().as_millis()
                );
            }

            Ok::<_, anyhow::Error>(())
        });

        handles.push(handle);
    }

    let results = futures::future::join_all(take(&mut handles)).await;
    let mut success_count = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (idx, r) in results.into_iter().enumerate() {
        match r {
            Ok(Ok(())) => success_count += 1,
            Ok(Err(e)) => failures.push(format!("batch={} err={}", idx, e)),
            Err(e) => failures.push(format!("batch={} join_err={}", idx, e)),
        }
    }
    if !failures.is_empty() {
        let preview = failures
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!(
            "gen_prim_geos_from_inputs 失败: success_batches={}, failed_batches={}, sample=[{}]",
            success_count,
            failures.len(),
            preview
        );
    }

    if is_e3d_debug_enabled() {
        println!(
            "[gen_prim_geos_from_inputs] 完成! 总数: {}, batch_success={}, 总耗时: {} ms",
            prim_cnt,
            success_count,
            t.elapsed().as_millis()
        );
    }
    Ok(true)
}
*/
