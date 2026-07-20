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
use aios_core::types::named_attvalue::NamedAttrValue;
use aios_core::{NamedAttrMap, SurrealQueryExt};
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet};
use std::mem::take;
use std::sync::Arc;
use std::sync::OnceLock;
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

static SPEC011_WATCH_REFNOS: OnceLock<Option<HashSet<u64>>> = OnceLock::new();

/// Env-gated probe for spec 011. Set `AIOS_SPEC011_REFNOS` to comma-separated
/// `ref0/ref1`, `ref0_ref1`, or raw u64 values to trace only those PRIMs.
fn spec011_watch_refnos() -> Option<&'static HashSet<u64>> {
    SPEC011_WATCH_REFNOS
        .get_or_init(|| {
            let raw = std::env::var("AIOS_SPEC011_REFNOS").ok()?;
            let set = raw
                .split(|c| matches!(c, ',' | ';' | ' ' | '\n' | '\r' | '\t'))
                .filter_map(parse_spec011_refno)
                .collect::<HashSet<_>>();
            (!set.is_empty()).then_some(set)
        })
        .as_ref()
}

fn parse_spec011_refno(token: &str) -> Option<u64> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Some((left, right)) = token.split_once('/').or_else(|| token.split_once('_')) {
        let ref0: u64 = left.trim().parse().ok()?;
        let ref1: u64 = right.trim().parse().ok()?;
        return Some((ref0 << 32) | ref1);
    }
    token.parse::<u64>().ok()
}

fn spec011_watch_hit(refno: RefnoEnum) -> bool {
    let Some(watch) = spec011_watch_refnos() else {
        return false;
    };
    watch.contains(&refno.refno().0)
}

fn spec011_log(refno: RefnoEnum, message: impl AsRef<str>) {
    if spec011_watch_hit(refno) {
        println!("[spec011][prim] refno={} {}", refno, message.as_ref());
    }
}

fn spec011_param_summary(attr: &aios_core::NamedAttrMap) -> String {
    const KEYS: &[&str] = &[
        "DIAM",
        "HEIG",
        "RADI",
        "DTOP",
        "DBOT",
        "XBOT",
        "YBOT",
        "XTOP",
        "YTOP",
        "XOFF",
        "YOFF",
        "DHEI",
        "SHEI",
        "DRAD",
        "SDIA",
        "SWID",
        "STHI",
        "SDIS",
        "DIMD",
        "PBTP",
        "PCTP",
        "PBBT",
        "PCBT",
        "PTDI",
        "PBDI",
        "PTOF",
        "PBOF",
        "PCOF",
        "_AIOS_TEMPLATE_Z_OFFSET",
        "_AIOS_TEMPLATE_ROT_Z",
    ];
    KEYS.iter()
        .filter_map(|key| attr.get_f32(key).map(|value| format!("{key}={value}")))
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_desp_index_expr(expr: &str, desp: &[f32]) -> Option<f32> {
    let upper = expr.to_ascii_uppercase();
    let Some(start) = upper.find("DESP[") else {
        return expr.trim().trim_matches('\'').parse::<f32>().ok();
    };
    let after = &upper[start + "DESP[".len()..];
    let end = after.find(']')?;
    let idx = after[..end].trim().parse::<usize>().ok()?;
    idx.checked_sub(1)
        .and_then(|zero_idx| desp.get(zero_idx).copied())
}

fn ddat_value_from_attr(attr: &NamedAttrMap, desp: &[f32]) -> Option<f32> {
    attr.get_as_string("DDPR")
        .as_deref()
        .and_then(|expr| parse_desp_index_expr(expr, desp))
        .or_else(|| {
            attr.get_as_string("DDDF")
                .as_deref()
                .and_then(|expr| parse_desp_index_expr(expr, desp))
        })
}

fn set_template_f32(attr: &mut NamedAttrMap, key: &str, value: f32) {
    if value.is_finite() {
        attr.map
            .insert(key.to_string(), NamedAttrValue::F32Type(value.max(0.0)));
    }
}

fn set_template_adjustment(attr: &mut NamedAttrMap, key: &str, value: f32) {
    if value.is_finite() {
        attr.map
            .insert(key.to_string(), NamedAttrValue::F32Type(value));
    }
}

fn apply_template_prim_params(
    cur_type: &str,
    params: &HashMap<String, f32>,
    attr: &mut NamedAttrMap,
) -> bool {
    let get = |key: &str| params.get(key).copied().filter(|v| v.is_finite());
    match cur_type {
        "CYLI" => {
            let Some(diam) = get("DIAM") else {
                return false;
            };
            let Some(total_height) = get("HEIG") else {
                return false;
            };
            let height =
                total_height - get("DHEI").unwrap_or_default() - get("SHEI").unwrap_or_default();
            set_template_f32(attr, "DIAM", diam);
            set_template_f32(attr, "HEIG", height);
            set_template_adjustment(attr, "_AIOS_TEMPLATE_Z_OFFSET", height * 0.5);
            height > f32::EPSILON
        }
        "DISH" => {
            let Some(diam) = get("DIAM") else {
                return false;
            };
            let Some(height) = get("DHEI").or_else(|| get("HEIG")) else {
                return false;
            };
            set_template_f32(attr, "DIAM", diam);
            set_template_f32(attr, "HEIG", height);
            if let Some(radius) = get("DRAD") {
                set_template_f32(attr, "RADI", radius);
            }
            if let Some(body_height) = get("HEIG") {
                set_template_adjustment(attr, "_AIOS_TEMPLATE_Z_OFFSET", body_height);
            }
            height > f32::EPSILON
        }
        "SNOU" => {
            let Some(height) = get("SHEI").or_else(|| get("HEIG")) else {
                return false;
            };
            let Some(top_diam) = get("DIAM") else {
                return false;
            };
            let Some(bottom_diam) = get("SDIA").or_else(|| get("DBOT")) else {
                return false;
            };
            set_template_f32(attr, "HEIG", height);
            set_template_f32(attr, "DTOP", top_diam);
            set_template_f32(attr, "DBOT", bottom_diam);
            set_template_f32(attr, "XOFF", 0.0);
            set_template_f32(attr, "YOFF", 0.0);
            set_template_adjustment(attr, "_AIOS_TEMPLATE_Z_OFFSET", -height * 0.5);
            height > f32::EPSILON
        }
        "PYRA" => {
            let Some(width) = get("SWID") else {
                return false;
            };
            let Some(thickness) = get("STHI") else {
                return false;
            };
            let Some(height) = get("SHEI").or_else(|| get("HEIG")) else {
                return false;
            };
            let span = get("DIMD").map(|value| value * 2.0).unwrap_or(width);
            set_template_f32(attr, "XBOT", thickness);
            set_template_f32(attr, "XTOP", thickness);
            set_template_f32(attr, "YBOT", span);
            set_template_f32(attr, "YTOP", (span - width).max(thickness));
            set_template_f32(attr, "HEIG", height);
            set_template_f32(attr, "XOFF", 0.0);
            set_template_f32(attr, "YOFF", 0.0);
            let z_offset = get("SDIS").unwrap_or_default() + height * 0.5;
            set_template_adjustment(attr, "_AIOS_TEMPLATE_Z_OFFSET", z_offset);
            span > f32::EPSILON && thickness > f32::EPSILON && height > f32::EPSILON
        }
        _ => false,
    }
}

fn apply_template_inst_adjustments(attr: &NamedAttrMap, inst: &mut EleInstGeo) {
    if let Some(z_offset) = attr.get_f32("_AIOS_TEMPLATE_Z_OFFSET") {
        inst.geo_transform.translation.z += z_offset;
    }
    if let Some(rotation_deg) = attr.get_f32("_AIOS_TEMPLATE_ROT_Z") {
        inst.geo_transform.rotation *= Quat::from_rotation_z(rotation_deg.to_radians());
    }
}

fn template_pyra_rotation_from_ori(attr: &NamedAttrMap) -> Option<f32> {
    let ori = attr.get_as_string("ORI")?.to_ascii_uppercase();
    if !ori.contains("Z IS U") {
        return None;
    }

    if ori.contains("Y IS N") {
        Some(0.0)
    } else if ori.contains("Y IS E") {
        Some(90.0)
    } else if ori.contains("Y IS S") {
        Some(180.0)
    } else if ori.contains("Y IS W") {
        Some(-90.0)
    } else {
        None
    }
}

fn attr_refno(attr: &NamedAttrMap, key: &str) -> Option<RefnoEnum> {
    match attr.map.get(key)? {
        NamedAttrValue::RefU64Type(value) => Some(RefnoEnum::Refno(*value)),
        NamedAttrValue::RefnoEnumType(value) => Some(*value),
        _ => None,
    }
}

async fn apply_template_primitive_orientation(cur_type: &str, attr: &mut NamedAttrMap) {
    if cur_type != "PYRA" {
        return;
    }
    if let Some(rotation_deg) = template_pyra_rotation_from_ori(attr) {
        set_template_adjustment(attr, "_AIOS_TEMPLATE_ROT_Z", rotation_deg);
        return;
    }

    let current_refno = attr
        .get_refno_or_default()
        .is_valid()
        .then(|| attr.get_refno_or_default());
    let owner_refno = attr.get_owner();
    if owner_refno.is_valid() {
        let siblings = crate::fast_model::query_provider::get_children(owner_refno)
            .await
            .unwrap_or_default();
        let mut same_type = Vec::new();
        for sibling in siblings {
            let sibling_type = aios_core::get_type_name(sibling).await.unwrap_or_default();
            if sibling_type == cur_type {
                same_type.push(sibling);
            }
        }
        if let Some(current_refno) = current_refno {
            if let Some(index) = same_type
                .iter()
                .position(|candidate| *candidate == current_refno)
            {
                let rotation_deg = if index % 2 == 0 { 0.0 } else { 90.0 };
                set_template_adjustment(attr, "_AIOS_TEMPLATE_ROT_Z", rotation_deg);
                return;
            }
        }
    }

    let Some(origin_refno) = attr_refno(attr, "ORRF") else {
        return;
    };
    let origin_attr = aios_core::get_named_attmap(origin_refno)
        .await
        .unwrap_or_default();
    let origin_owner = origin_attr.get_owner();
    if !origin_owner.is_valid() {
        return;
    }
    let siblings = crate::fast_model::query_provider::get_children(origin_owner)
        .await
        .unwrap_or_default();
    let mut same_type = Vec::new();
    for sibling in siblings {
        let sibling_type = aios_core::get_type_name(sibling).await.unwrap_or_default();
        if sibling_type == cur_type {
            same_type.push(sibling);
        }
    }
    let Some(index) = same_type
        .iter()
        .position(|candidate| *candidate == origin_refno)
    else {
        return;
    };
    let rotation_deg = if index % 2 == 0 { 180.0 } else { -90.0 };
    set_template_adjustment(attr, "_AIOS_TEMPLATE_ROT_Z", rotation_deg);
}

async fn resolve_template_prim_attr(refno: RefnoEnum, attr: NamedAttrMap) -> NamedAttrMap {
    let cur_type = attr.get_type_str().to_string();
    if !matches!(cur_type.as_str(), "CYLI" | "DISH" | "SNOU" | "PYRA") {
        return attr;
    }

    let tmpl_refno = attr.get_owner();
    spec011_log(
        refno,
        format!("template_resolve_start type={cur_type} owner={tmpl_refno}"),
    );
    if !tmpl_refno.is_valid() {
        spec011_log(refno, "template_resolve_stop=invalid_template_owner");
        return attr;
    }
    let tmpl_attr = aios_core::get_named_attmap(tmpl_refno)
        .await
        .unwrap_or_default();
    if tmpl_attr.get_type_str() != "TMPL" {
        spec011_log(
            refno,
            format!(
                "template_resolve_stop=owner_not_tmpl owner_type={}",
                tmpl_attr.get_type_str()
            ),
        );
        return attr;
    }

    let design_owner = tmpl_attr.get_owner();
    if !design_owner.is_valid() {
        spec011_log(refno, "template_resolve_stop=invalid_design_owner");
        return attr;
    }
    let design_attr = aios_core::get_named_attmap(design_owner)
        .await
        .unwrap_or_default();
    let Some(desp) = design_attr
        .get_f32_vec("DESP")
        .filter(|values| !values.is_empty())
    else {
        spec011_log(
            refno,
            format!(
                "template_resolve_stop=missing_desp design_owner={} design_type={}",
                design_owner,
                design_attr.get_type_str()
            ),
        );
        return attr;
    };

    let tmpl_children = crate::fast_model::query_provider::get_children(tmpl_refno)
        .await
        .unwrap_or_default();
    spec011_log(
        refno,
        format!(
            "template_resolve_desp={:?} tmpl_children={}",
            desp,
            tmpl_children.len()
        ),
    );
    let mut ddat_owner_refno = None;
    let mut ddat_fallback_owner_refno = None;
    for child in tmpl_children {
        let child_attr = aios_core::get_named_attmap(child).await.unwrap_or_default();
        if child_attr.get_type_str() == "DDSE" {
            ddat_owner_refno = Some(child);
            ddat_fallback_owner_refno = attr_refno(&child_attr, "ORRF");
            break;
        }
    }
    let Some(mut ddat_owner_refno) = ddat_owner_refno else {
        spec011_log(refno, "template_resolve_stop=missing_ddse");
        return attr;
    };

    let mut ddat_refnos = crate::fast_model::query_provider::get_children(ddat_owner_refno)
        .await
        .unwrap_or_default();
    if ddat_refnos.is_empty() {
        if let Some(fallback_owner) = ddat_fallback_owner_refno {
            ddat_owner_refno = fallback_owner;
            ddat_refnos = crate::fast_model::query_provider::get_children(fallback_owner)
                .await
                .unwrap_or_default();
        }
    }
    let mut params = HashMap::new();
    for ddat_refno in &ddat_refnos {
        let ddat_attr = aios_core::get_named_attmap(*ddat_refno)
            .await
            .unwrap_or_default();
        if ddat_attr.get_type_str() != "DDAT" {
            continue;
        }
        let Some(key) = ddat_attr
            .get_as_string("DKEY")
            .map(|key| key.trim().to_uppercase())
        else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        if let Some(value) = ddat_value_from_attr(&ddat_attr, &desp) {
            params.insert(key, value);
        }
    }
    spec011_log(
        refno,
        format!(
            "template_resolve_ddat_owner={} ddat_count={} params={:?}",
            ddat_owner_refno,
            ddat_refnos.len(),
            params
        ),
    );

    let mut resolved = attr;
    if apply_template_prim_params(&cur_type, &params, &mut resolved) {
        apply_template_primitive_orientation(&cur_type, &mut resolved).await;
        spec011_log(
            refno,
            format!(
                "template_params_applied type={} owner={} design_owner={} params={}",
                cur_type,
                tmpl_refno,
                design_owner,
                spec011_param_summary(&resolved)
            ),
        );
    } else {
        spec011_log(
            refno,
            format!("template_resolve_stop=required_params_missing type={cur_type}"),
        );
    }
    resolved
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
        spec011_log(refno, "skip_detail=shape_check_valid_false");
        return None;
    }

    let mut transform = csg_shape.get_trans();
    if transform.translation.is_nan() || transform.rotation.is_nan() || transform.scale.is_nan() {
        spec011_log(
            refno,
            format!(
                "skip_detail=transform_nan translation_nan={} rotation_nan={} scale_nan={}",
                transform.translation.is_nan(),
                transform.rotation.is_nan(),
                transform.scale.is_nan()
            ),
        );
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

fn build_datum_marker_geos(refno: RefnoEnum, visible: bool) -> Vec<EleInstGeo> {
    [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .filter_map(|direction| {
            let shape: Box<dyn BrepShapeTrait> = Box::new(SCylinder {
                phei: 100.0,
                pdia: 10.0,
                center_in_mid: false,
                ..Default::default()
            });
            let mut inst = build_inst_geo_from_shape(shape, refno, visible, false)?;
            inst.geo_transform.rotation = Quat::from_rotation_arc(Vec3::Z, direction);
            inst.geo_transform.translation = direction * -50.0;
            Some(inst)
        })
        .collect()
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
    insts: Vec<EleInstGeo>,
    neg_refnos: &[RefnoEnum],
    type_name: &str,
) {
    let refno = geos_info.refno;
    let is_solid = insts.iter().any(|inst| inst.geo_type == GeoBasicType::Pos);
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
            insts,
            aabb: None,
            type_name: type_name.to_string(),
        },
    );
    shape_insts_data.insert_info(refno, geos_info);
}

/// 如果 batch 达到阈值则发送，返回是否发送成功。
async fn flush_if_needed(
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
        let inst_cnt = shape_insts_data.inst_cnt();
        let send_start = Instant::now();
        sender
            .send_async(std::mem::take(shape_insts_data))
            .await
            .map_err(|e| anyhow::anyhow!("send prim shape_insts_data error: {}", e))?;
        println!(
            "[producer_batch] producer=prim batch={} flush=threshold inst_cnt={} send_wait_ms={}",
            batch_idx,
            inst_cnt,
            send_start.elapsed().as_millis()
        );
        *sent_count += 1;
    }
    Ok(())
}

/// 发送剩余数据。
async fn flush_remaining(
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
        let inst_cnt = shape_insts_data.inst_cnt();
        let send_start = Instant::now();
        sender
            .send_async(shape_insts_data)
            .await
            .map_err(|e| anyhow::anyhow!("send last prim shape_insts_data error: {}", e))?;
        println!(
            "[producer_batch] producer=prim batch={} flush=final inst_cnt={} send_wait_ms={}",
            batch_idx,
            inst_cnt,
            send_start.elapsed().as_millis()
        );
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

    if let Some(watch) = spec011_watch_refnos() {
        let hits = prim_refnos
            .iter()
            .copied()
            .filter(|refno| watch.contains(&refno.refno().0))
            .collect::<Vec<_>>();
        println!(
            "[spec011][prim] page_input total={} watched_hits={:?}",
            prim_cnt, hits
        );
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
                let tree_dir = db_option.get_scene_tree_dir();
                neg_query::query_descendants_map_by_dbnum_dual(
                    &tree_dir,
                    &batch_refnos,
                    &GENRAL_NEG_NOUN_NAMES,
                    false,
                )
                .await
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
                    spec011_log(refno, "skip=world_transform_missing");
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
                let attr = resolve_template_prim_attr(refno, attr).await;
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

                let datum_geos = matches!(cur_type, "JLDATU" | "PLDATU")
                    .then(|| build_datum_marker_geos(refno, visible));
                let csg_shape = if datum_geos.is_some() {
                    None
                } else if cur_type == "POHE" || cur_type == "POLYHE" {
                    build_polyhedron_from_db(refno).await
                } else {
                    attr.create_csg_shape(neg_limit_size)
                };

                let inst_geos = if let Some(insts) = datum_geos {
                    insts
                } else {
                    let Some(csg_shape) = csg_shape else {
                        skipped_in_batch += 1;
                        spec011_log(
                            refno,
                            format!(
                                "skip=create_csg_shape_failed type={cur_type} visible={visible}"
                            ),
                        );
                        continue;
                    };
                    let Some(mut inst_geo) =
                        build_inst_geo_from_shape(csg_shape, refno, visible, attr.is_neg())
                    else {
                        skipped_in_batch += 1;
                        spec011_log(
                            refno,
                            format!(
                                "skip=invalid_inst_geo type={cur_type} params={}",
                                spec011_param_summary(&attr)
                            ),
                        );
                        continue;
                    };
                    apply_template_inst_adjustments(&attr, &mut inst_geo);
                    vec![inst_geo]
                };

                let neg_refnos = neg_map.get(&refno).cloned().unwrap_or_default();

                insert_prim_result(
                    &mut shape_insts_data,
                    geos_info,
                    inst_geos,
                    &neg_refnos,
                    cur_type,
                );
                processed_in_batch += 1;
                spec011_log(
                    refno,
                    format!(
                        "inserted type={} visible={} is_neg={} batch={}",
                        cur_type,
                        visible,
                        attr.is_neg(),
                        i
                    ),
                );

                flush_if_needed(&mut shape_insts_data, &sender, i, &mut sent_count).await?;
            }

            flush_remaining(shape_insts_data, &sender, i, &mut sent_count).await?;

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

                let datum_geos = matches!(cur_type, "JLDATU" | "PLDATU")
                    .then(|| build_datum_marker_geos(refno, visible));
                let csg_shape: Option<Box<dyn BrepShapeTrait>> =
                    if datum_geos.is_some() {
                        None
                    } else if cur_type == "POHE" || cur_type == "POLYHE" {
                        input.poly_extra.as_ref().map(build_polyhedron_from_cache)
                    } else {
                        attr.create_csg_shape(neg_limit_size)
                    };

                let inst_geos = if let Some(insts) = datum_geos {
                    insts
                } else {
                    let Some(csg_shape) = csg_shape else {
                        skipped_in_batch += 1;
                        continue;
                    };
                    let Some(mut inst_geo) =
                        build_inst_geo_from_shape(csg_shape, refno, visible, attr.is_neg())
                    else {
                        skipped_in_batch += 1;
                        continue;
                    };
                    apply_template_inst_adjustments(attr, &mut inst_geo);
                    vec![inst_geo]
                };

                // 插入结果
                insert_prim_result(
                    &mut shape_insts_data,
                    geos_info,
                    inst_geos,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn attr_with_type(noun: &str) -> NamedAttrMap {
        let mut attr = NamedAttrMap::default();
        attr.map.insert(
            "TYPE".to_string(),
            NamedAttrValue::StringType(noun.to_string()),
        );
        attr
    }

    fn f32_attr(attr: &NamedAttrMap, key: &str) -> f32 {
        attr.get_f32(key)
            .unwrap_or_else(|| panic!("missing f32 attr {key}"))
    }

    #[test]
    fn spec011_desp_index_expr_reads_one_based_desp_values() {
        let desp = [1200.0, 800.0, 200.0, 100.0, 600.0, 80.0, 150.0];

        assert_eq!(parse_desp_index_expr("DESP[1]", &desp), Some(1200.0));
        assert_eq!(parse_desp_index_expr("'DESP[7]'", &desp), Some(150.0));
        assert_eq!(parse_desp_index_expr("42.5", &desp), Some(42.5));
        assert_eq!(parse_desp_index_expr("DESP[0]", &desp), None);
        assert_eq!(parse_desp_index_expr("DESP[99]", &desp), None);
    }

    #[test]
    fn spec011_template_params_fill_cyli_dish_and_snou_fields() {
        let params = HashMap::from([
            ("DIAM".to_string(), 800.0),
            ("HEIG".to_string(), 1200.0),
            ("DHEI".to_string(), 200.0),
            ("DRAD".to_string(), 100.0),
            ("SHEI".to_string(), 600.0),
            ("SDIA".to_string(), 150.0),
        ]);

        let mut cyli = attr_with_type("CYLI");
        assert!(apply_template_prim_params("CYLI", &params, &mut cyli));
        assert_eq!(f32_attr(&cyli, "DIAM"), 800.0);
        assert_eq!(f32_attr(&cyli, "HEIG"), 400.0);
        assert_eq!(f32_attr(&cyli, "_AIOS_TEMPLATE_Z_OFFSET"), 200.0);

        let mut dish = attr_with_type("DISH");
        assert!(apply_template_prim_params("DISH", &params, &mut dish));
        assert_eq!(f32_attr(&dish, "DIAM"), 800.0);
        assert_eq!(f32_attr(&dish, "HEIG"), 200.0);
        assert_eq!(f32_attr(&dish, "RADI"), 100.0);
        assert_eq!(f32_attr(&dish, "_AIOS_TEMPLATE_Z_OFFSET"), 1200.0);

        let mut snou = attr_with_type("SNOU");
        assert!(apply_template_prim_params("SNOU", &params, &mut snou));
        assert_eq!(f32_attr(&snou, "HEIG"), 600.0);
        assert_eq!(f32_attr(&snou, "DTOP"), 800.0);
        assert_eq!(f32_attr(&snou, "DBOT"), 150.0);
        assert_eq!(f32_attr(&snou, "XOFF"), 0.0);
        assert_eq!(f32_attr(&snou, "YOFF"), 0.0);
        assert_eq!(f32_attr(&snou, "_AIOS_TEMPLATE_Z_OFFSET"), -300.0);
    }

    #[test]
    fn spec011_template_params_fill_pyra_fields() {
        let params = HashMap::from([
            ("SWID".to_string(), 600.0),
            ("STHI".to_string(), 80.0),
            ("SHEI".to_string(), 150.0),
            ("DIMD".to_string(), 400.0),
            ("SDIS".to_string(), 25.0),
        ]);

        let mut pyra = attr_with_type("PYRA");
        assert!(apply_template_prim_params("PYRA", &params, &mut pyra));
        assert_eq!(f32_attr(&pyra, "XBOT"), 80.0);
        assert_eq!(f32_attr(&pyra, "XTOP"), 80.0);
        assert_eq!(f32_attr(&pyra, "YBOT"), 800.0);
        assert_eq!(f32_attr(&pyra, "YTOP"), 200.0);
        assert_eq!(f32_attr(&pyra, "HEIG"), 150.0);
        assert_eq!(f32_attr(&pyra, "XOFF"), 0.0);
        assert_eq!(f32_attr(&pyra, "YOFF"), 0.0);
        assert_eq!(f32_attr(&pyra, "_AIOS_TEMPLATE_Z_OFFSET"), 100.0);
    }

    #[test]
    fn spec011_template_params_reject_missing_required_values() {
        let mut cyli = attr_with_type("CYLI");
        assert!(!apply_template_prim_params(
            "CYLI",
            &HashMap::from([("DIAM".to_string(), 800.0)]),
            &mut cyli,
        ));

        let mut pyra = attr_with_type("PYRA");
        assert!(!apply_template_prim_params(
            "PYRA",
            &HashMap::from([("SWID".to_string(), 600.0), ("STHI".to_string(), 80.0),]),
            &mut pyra,
        ));
    }

    #[test]
    fn datum_marker_geos_emit_three_visible_positive_axes() {
        let refno = RefnoEnum::default();
        let geos = build_datum_marker_geos(refno, true);

        assert_eq!(geos.len(), 3);
        assert!(geos.iter().all(|geo| geo.refno == refno));
        assert!(geos.iter().all(|geo| geo.visible));
        assert!(geos.iter().all(|geo| geo.geo_type == GeoBasicType::Pos));
        assert!(
            geos.iter()
                .all(|geo| geo.geo_transform.translation.is_finite())
        );
        assert_eq!(geos[0].geo_transform.translation, Vec3::X * -50.0);
        assert_eq!(geos[1].geo_transform.translation, Vec3::Y * -50.0);
        assert_eq!(geos[2].geo_transform.translation, Vec3::Z * -50.0);
    }
}
