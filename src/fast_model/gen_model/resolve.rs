use crate::expression_fix::ExpressionFixer;
use crate::fast_model::gen_model::{GenerationReadContext, session_query};
use crate::fast_model::query_gm_params;
use crate::fast_model::{debug_model, debug_model_debug, debug_model_trace};
use aios_core::SurrealQueryExt;
use aios_core::consts::WORD_HASH;
use aios_core::expression::query_cata::{get_axis_param, query_axis_params, resolve_cata_comp};
use aios_core::expression::resolve::{SCOM_INFO_MAP, resolve_axis_param};
use aios_core::parsed_data::{CateAxisParam, CateGeomsInfo};
use aios_core::pdms_data::{GmParam, PlinParam, ScomInfo};
use aios_core::pdms_types::TOTAL_CATA_GEO_NOUN_NAMES;
use aios_core::{CataContext, NamedAttrMap, NamedAttrValue, RefU64, RefnoEnum, project_primary_db};
use anyhow::anyhow;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

fn resolve_trace_refno_filter() -> Option<String> {
    std::env::var("AIOS_CATA_P1_TRACE_REFNO")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn should_trace_resolve_desi(desi_refno: RefnoEnum) -> bool {
    let Some(target) = resolve_trace_refno_filter() else {
        return false;
    };
    let target_normalized = target.replace('/', "_");
    target == desi_refno.to_string()
        || target_normalized == desi_refno.to_string()
        || target == desi_refno.to_e3d_id()
}

fn normalize_gm_param_expressions_in_place(gm: &mut GmParam) {
    // 仅做“去掉 ATTRIB :NAME 中的冒号”这种低风险规整，避免 aios_core 表达式解析器直接拒绝。
    // 额外规整少量历史前缀表达式（如 `TWICE PARAM 3`），避免元件库求值阶段直接丢几何。
    // 不做更激进的重写（例如移除 ATTRIB 或把 [n] 展平），以降低行为回归风险。
    let normalize_expr = |expr: &str| {
        let expr = ExpressionFixer::normalize_attrib_colon(expr);
        ExpressionFixer::normalize_pdms_prefix_operators(&expr)
    };

    gm.prad = normalize_expr(&gm.prad);
    gm.pang = normalize_expr(&gm.pang);
    gm.pwid = normalize_expr(&gm.pwid);
    gm.phei = normalize_expr(&gm.phei);
    gm.offset = normalize_expr(&gm.offset);
    gm.drad = normalize_expr(&gm.drad);
    gm.dwid = normalize_expr(&gm.dwid);

    for expr in gm.diameters.iter_mut() {
        *expr = normalize_expr(expr);
    }
    for expr in gm.distances.iter_mut() {
        *expr = normalize_expr(expr);
    }
}

/// 查询 DESI 元素的 IPARAM 数据
/// 使用 SurrealDB 的 fn::get_ipara 函数
async fn query_iparam_from_desi(desi_refno: RefnoEnum) -> anyhow::Result<Vec<f32>> {
    let sql = format!("return fn::get_ipara({})", desi_refno.to_pe_key());
    let result: Vec<f32> = project_primary_db().query_take(&sql, 0).await?;

    Ok(result)
}

async fn query_iparam_from_session(
    read: &GenerationReadContext,
    desi_att: &NamedAttrMap,
) -> anyhow::Result<Vec<f32>> {
    let Some(ispe_refno) = desi_att.get_foreign_refno("ISPE") else {
        return Ok(Vec::new());
    };
    for spco_refno in session_query::get_children(read, ispe_refno) {
        if !matches!(
            session_query::get_type_name(read, spco_refno).as_deref(),
            Ok("SPCO")
        ) {
            continue;
        }
        let spco = session_query::get_named_attmap(read, spco_refno).await?;
        let Some(catr_refno) = spco.get_foreign_refno("CATR") else {
            continue;
        };
        return Ok(session_query::get_named_attmap(read, catr_refno)
            .await?
            .get_f32_vec("PARA")
            .unwrap_or_default());
    }
    Ok(Vec::new())
}

async fn query_gm_param_from_session(
    read: &GenerationReadContext,
    att: &NamedAttrMap,
) -> anyhow::Result<GmParam> {
    let mut paxises = att.get_attr_strings_without_default(&["PAXI", "PAAX", "PBAX", "PCAX"]);
    if let Some(NamedAttrValue::IntArrayType(values)) = att.get_val("PTS") {
        paxises.extend(values.iter().map(ToString::to_string));
    }
    if let Some(value) = att.get_as_string("PLAX") {
        paxises.push(value);
    }

    let refno = att.get_refno_or_default();
    let type_name = att.get_type_str();
    let mut verts = Vec::new();
    let mut frads = Vec::new();
    let mut dxy = Vec::new();
    if matches!(type_name, "SEXT" | "NSEX" | "SREV" | "NSRE") {
        for child in session_query::get_children(read, refno) {
            if !matches!(
                session_query::get_type_name(read, child).as_deref(),
                Ok("SLOO")
            ) {
                continue;
            }
            let vertices = session_query::get_children(read, child);
            for vertex in session_query::get_named_attmaps(read, &vertices)
                .await?
                .into_values()
            {
                verts.push([
                    vertex.get_as_string("PX").unwrap_or_default(),
                    vertex.get_as_string("PY").unwrap_or_default(),
                    vertex.get_as_string("PZ").unwrap_or_default(),
                ]);
                frads.push(vertex.get_as_string("PRAD").unwrap_or_default());
            }
        }
    } else if type_name == "SPRO" {
        let children = session_query::get_children(read, refno);
        for child in session_query::get_named_attmaps(read, &children)
            .await?
            .into_values()
        {
            if matches!(child.get_type_str(), "SPVE" | "SVER" | "PVER") {
                verts.push([
                    child.get_as_string("PX").unwrap_or_default(),
                    child.get_as_string("PY").unwrap_or_default(),
                    child.get_as_string("PZ").unwrap_or_default(),
                ]);
                frads.push(child.get_as_string("PRAD").unwrap_or_default());
                dxy.push([
                    child.get_as_string("DX").unwrap_or_default(),
                    child.get_as_string("DY").unwrap_or_default(),
                ]);
            }
        }
    } else {
        verts.push([
            att.get_as_string("PX").unwrap_or_default(),
            att.get_as_string("PY").unwrap_or_default(),
            att.get_as_string("PZ").unwrap_or_default(),
        ]);
        frads.push(att.get_as_string("PRAD").unwrap_or_default());
        dxy.push([
            att.get_as_string("DX").unwrap_or_default(),
            att.get_as_string("DY").unwrap_or_default(),
        ]);
    }

    Ok(GmParam {
        refno,
        gm_type: type_name.to_owned(),
        prad: att.get_as_string("PRAD").unwrap_or_default(),
        pang: att.get_as_string("PANG").unwrap_or_default(),
        pwid: att.get_as_string("PWID").unwrap_or_default(),
        diameters: att.get_attr_strings(&["PDIA", "PBDM", "PTDM", "DIAM"]),
        distances: att.get_attr_strings(&["PDIS", "PBDI", "PTDI"]),
        shears: att.get_attr_strings(&["PXTS", "PYTS", "PXBS", "PYBS"]),
        phei: att.get_as_string("PHEI").unwrap_or_default(),
        offset: att.get_as_string("POFF").unwrap_or_default(),
        lengths: att.get_attr_strings(&["PXLE", "PYLE", "PZLE"]),
        xyz: att.get_attr_strings(&[
            "PX", "PY", "PZ", "PBBT", "PCBT", "PBTP", "PCTP", "PBOF", "PCOF",
        ]),
        verts,
        frads,
        dxy,
        drad: att.get_as_string("DRAD").unwrap_or_default(),
        dwid: att.get_as_string("DWID").unwrap_or_default(),
        paxises,
        centre_line_flag: att.get_bool("CLFL").unwrap_or(false),
        visible_flag: att.get_bool("TUFL").unwrap_or(true),
        plax: att.get_as_string("PLAX"),
    })
}

async fn query_gm_params_from_session(
    read: &GenerationReadContext,
    root: RefnoEnum,
) -> anyhow::Result<Vec<GmParam>> {
    let mut refnos = session_query::get_children(read, root);
    let nested = refnos
        .iter()
        .flat_map(|refno| session_query::get_children(read, *refno))
        .collect::<Vec<_>>();
    refnos.extend(nested);
    let mut attributes = session_query::get_named_attmaps(read, &refnos).await?;
    let mut result = Vec::new();
    for refno in refnos {
        let Some(attribute) = attributes.remove(&refno) else {
            continue;
        };
        if !TOTAL_CATA_GEO_NOUN_NAMES.contains(&attribute.get_type_str())
            || !attribute.is_visible_by_level(None).unwrap_or(true)
        {
            continue;
        }
        result.push(query_gm_param_from_session(read, &attribute).await?);
    }
    Ok(result)
}

async fn get_or_create_scom_info_from_session(
    read: &GenerationReadContext,
    cata_refno: RefnoEnum,
) -> anyhow::Result<ScomInfo> {
    let attr_map = session_query::get_named_attmap(read, cata_refno).await?;
    let ptref_name = if attr_map.get_type_str() == "SPRF" {
        "PSTR"
    } else {
        "PTRE"
    };
    let mut axis_params = Vec::new();
    let mut axis_param_numbers = Vec::new();
    if let Some(ptre_refno) = attr_map.get_foreign_refno(ptref_name) {
        let children = session_query::get_children(read, ptre_refno);
        for child in session_query::get_named_attmaps(read, &children)
            .await?
            .into_values()
        {
            if child.get_type_str() == "PLIN" {
                continue;
            }
            if let Some(axis) = get_axis_param(&child) {
                axis_param_numbers.push(child.get_i32("NUMB").unwrap_or(-1));
                axis_params.push(axis);
            }
        }
    }

    let gmse_refno = session_query::first_outbound_reference(read, cata_refno, &["GMRE", "GSTR"])
        .await?
        .ok_or_else(|| anyhow::anyhow!("SCOM {} missing GMRE/GSTR", cata_refno))?;
    let mut gm_params = query_gm_params_from_session(read, gmse_refno).await?;
    for gm in &mut gm_params {
        normalize_gm_param_expressions_in_place(gm);
    }
    let mut ngm_params = Vec::new();
    if let Some(ngmr_refno) = attr_map.get_foreign_refno("NGMR") {
        ngm_params = query_gm_params_from_session(read, ngmr_refno).await?;
        for gm in &mut ngm_params {
            normalize_gm_param_expressions_in_place(gm);
        }
    }

    let mut plin_map = HashMap::new();
    if let Some(pstr_refno) = attr_map.get_foreign_refno("PSTR") {
        let children = session_query::get_children(read, pstr_refno);
        for attribute in session_query::get_named_attmaps(read, &children)
            .await?
            .into_values()
        {
            if let Some(key) = attribute.get_as_string("PKEY") {
                plin_map.insert(
                    key,
                    PlinParam {
                        vxy: [
                            attribute.get_as_string("PX").unwrap_or_else(|| "0".into()),
                            attribute.get_as_string("PY").unwrap_or_else(|| "0".into()),
                        ],
                        dxy: [
                            attribute.get_as_string("DX").unwrap_or_else(|| "0".into()),
                            attribute.get_as_string("DY").unwrap_or_else(|| "0".into()),
                        ],
                        plax: attribute
                            .get_as_string("PLAX")
                            .unwrap_or_else(|| "unset".into()),
                    },
                );
            }
        }
    }

    Ok(ScomInfo {
        gtype: attr_map
            .get_as_string("GTYP")
            .unwrap_or_else(|| "unset".into()),
        dtse_params: Vec::new(),
        gm_params,
        ngm_params,
        axis_params,
        params: attr_map
            .get_as_string("PARA")
            .unwrap_or_default()
            .replace('\n', " ")
            .replace("  ", " ")
            .into(),
        axis_param_numbers,
        attr_map,
        plin_map,
    })
}

fn insert_iparam_kv(context: &mut CataContext, idx1: usize, v: &str) {
    // 历史表达式里 IPARAM/IPARA/IPAR/IPARM 的写法不统一，这里全量铺开，避免漏键导致表达式求值失败。
    context.insert(format!("IPARAM {}", idx1), v.to_string());
    context.insert(format!("IPARAM{}", idx1), v.to_string());
    context.insert(format!("IPARA {}", idx1), v.to_string());
    context.insert(format!("IPARA{}", idx1), v.to_string());
    context.insert(format!("IPAR {}", idx1), v.to_string());
    context.insert(format!("IPAR{}", idx1), v.to_string());
    context.insert(format!("IPARM {}", idx1), v.to_string());
    context.insert(format!("IPARM{}", idx1), v.to_string());
}

/// 命中未解析 CATA refno 时的惰性兜底（spec 002 T007）。
///
/// 仅在 `AIOS_CATA_CLOSURE_MODE=manifest`（部分解析模式）下生效：记 cache-miss →
/// [`crate::data_interface::cata_closure::ensure_cata_refnos_parsed`] 小闭包解析落库 →
/// 返回是否值得重试原查询。整库解析模式下 miss 即真缺数据，不兜底。
#[cfg(all(feature = "sqlite-index", feature = "surreal-save"))]
async fn try_lazy_cata_fallback(cata_refno: RefnoEnum, stage: &'static str) -> bool {
    use crate::data_interface::cata_closure::{
        CataClosureSyncMode, cata_closure_sync_mode, ensure_cata_refnos_parsed,
    };

    if cata_closure_sync_mode() != CataClosureSyncMode::Manifest {
        return false;
    }
    super::cache_miss_report::with_global_report(|report| {
        report.record_refno_miss(stage, "cata_refno_unparsed_lazy_fallback", cata_refno, None);
    });
    match ensure_cata_refnos_parsed(&[cata_refno.refno()]).await {
        Ok(outcome) if outcome.parsed > 0 => {
            log::info!(
                "[cata_closure] 惰性兜底成功({stage}): {} 解析落库 {} 个元素（missing={}），重试原查询",
                cata_refno,
                outcome.parsed,
                outcome.missing
            );
            true
        }
        Ok(outcome) => {
            log::warn!(
                "[cata_closure] 惰性兜底未解析到元素({stage}): {}（missing={}）",
                cata_refno,
                outcome.missing
            );
            false
        }
        Err(e) => {
            log::warn!(
                "[cata_closure] 惰性兜底失败({stage}): {}: {}",
                cata_refno,
                e
            );
            false
        }
    }
}

#[cfg(not(all(feature = "sqlite-index", feature = "surreal-save")))]
async fn try_lazy_cata_fallback(_cata_refno: RefnoEnum, _stage: &'static str) -> bool {
    false
}

///收集SCOM的信息, 暂时慎用缓存
pub async fn get_or_create_scom_info(cata_refno: RefnoEnum) -> anyhow::Result<ScomInfo> {
    // P5 优化：禁用 debug 模式下的缓存清除，避免重复查询相同 SCOM
    // 原逻辑：每次调用都清除缓存，导致 P4 预取时 6502 个子元素重复查询 ~10 个唯一 SCOM
    // 优化后：SCOM 缓存全局有效，6502 次调用 → 只有 ~10 次实际 DB 查询
    // if aios_core::is_debug_model_enabled() {
    //     SCOM_INFO_MAP.remove(&cata_refno);
    //     debug_model_debug!("Cleared SCOM_INFO_MAP cache for {}", cata_refno);
    // }

    let scom_info = if let Some(info) = SCOM_INFO_MAP.get(&cata_refno) {
        info.value().clone()
    } else {
        // 命中未解析（部分解析模式下 pe 缺失）→ 惰性兜底小闭包后重试一次（spec 002 T007）。
        let attr_map = match aios_core::get_named_attmap(cata_refno).await {
            Ok(attr_map) => attr_map,
            Err(first_err) => {
                if try_lazy_cata_fallback(cata_refno, "get_or_create_scom_info").await {
                    aios_core::get_named_attmap(cata_refno).await?
                } else {
                    return Err(first_err);
                }
            }
        };
        let type_noun = attr_map.get_type_str();
        let ptref_name = match type_noun {
            "SPRF" => "PSTR",
            _ => "PTRE",
        };
        let mut axis_params = vec![];
        let mut axis_param_numbers = vec![];
        if let Some(ptre_refno) = attr_map.get_foreign_refno(ptref_name) {
            if let Ok(axis_param_map) = query_axis_params(ptre_refno).await {
                axis_params = axis_param_map.values().cloned().collect::<Vec<_>>();
                axis_param_numbers = axis_param_map.keys().cloned().collect::<Vec<_>>();
            }
        }
        let gmse_refno =
            aios_core::query_single_by_paths(cata_refno, &["->GMRE", "->GSTR"], &["REFNO"])
                .await
                .map(|x| x.get_refno_or_default())?;
        debug_model_trace!("gmse_refno: {:?}", gmse_refno);
        let mut gm_params = query_gm_params(gmse_refno).await?;
        for gm in gm_params.iter_mut() {
            normalize_gm_param_expressions_in_place(gm);
        }
        let mut ngm_params = vec![];
        //-ve， 和design发生左右的负实体
        if let Some(gmse_refno) = attr_map.get_foreign_refno("NGMR") {
            ngm_params = query_gm_params(gmse_refno).await?;
            for gm in ngm_params.iter_mut() {
                normalize_gm_param_expressions_in_place(gm);
            }
        }

        let mut plin_map = HashMap::new();
        if let Some(pstr_refno) = attr_map.get_foreign_refno("PSTR") {
            // 使用新的泛型函数接口
            let pstr_am = aios_core::collect_children_filter_attrs(pstr_refno, &[]).await?;
            for a in pstr_am {
                if let Some(k) = a.get_as_string("PKEY") {
                    plin_map.insert(
                        k,
                        PlinParam {
                            vxy: [
                                a.get_as_string("PX").unwrap_or("0".to_string()),
                                a.get_as_string("PY").unwrap_or("0".to_string()),
                            ],
                            dxy: [
                                a.get_as_string("DX").unwrap_or("0".to_string()),
                                a.get_as_string("DY").unwrap_or("0".to_string()),
                            ],
                            plax: a.get_as_string("PLAX").unwrap_or("unset".to_string()),
                        },
                    );
                }
            }
        }
        ScomInfo {
            gtype: attr_map.get_as_string("GTYP").unwrap_or("unset".into()),
            dtse_params: vec![],
            gm_params,
            ngm_params,
            axis_params,
            params: attr_map
                .get_as_string("PARA")
                .unwrap_or_default()
                .replace("\n", " ")
                .replace("  ", " ")
                .into(),
            axis_param_numbers,
            attr_map,
            plin_map,
        }
    };
    Ok(scom_info)
}

/// 求解axis的数值
pub async fn resolve_axis_params(
    refno: RefnoEnum,
    context: Option<CataContext>,
) -> anyhow::Result<BTreeMap<i32, CateAxisParam>> {
    let mut map = BTreeMap::new();
    let scom_refno = aios_core::get_cat_refno(refno).await?.unwrap_or_default();
    if !scom_refno.is_valid() {
        return Ok(Default::default());
    }
    let scom = get_or_create_scom_info(scom_refno).await?;
    let context = context.unwrap_or(aios_core::get_or_create_cata_context(refno, false).await?);
    for i in 0..scom.axis_params.len() {
        let axis = resolve_axis_param(&scom.axis_params[i], &scom, &context);
        map.insert(scom.axis_param_numbers[i], axis);
    }
    Ok(map)
}

async fn normalize_catalog_scom_ref(mut catalog_ref: RefnoEnum) -> RefnoEnum {
    for _ in 0..4 {
        if !catalog_ref.is_valid() {
            break;
        }
        let Ok(attr) = aios_core::get_named_attmap(catalog_ref).await else {
            break;
        };
        let type_name = attr.get_type_str();
        if matches!(type_name, "SCOM" | "SPRF" | "SFIT" | "JOIN") {
            break;
        }
        let Some(next_ref) = attr.get_foreign_refno("CATR") else {
            break;
        };
        if !next_ref.is_valid() || next_ref == catalog_ref {
            break;
        }
        debug_model_trace!(
            "normalize catalog ref via CATR: from={:?} type={} to={:?}",
            catalog_ref,
            type_name,
            next_ref
        );
        catalog_ref = next_ref;
    }
    catalog_ref
}

async fn normalize_catalog_scom_ref_from_session(
    read: &GenerationReadContext,
    mut catalog_ref: RefnoEnum,
) -> RefnoEnum {
    for _ in 0..4 {
        if !catalog_ref.is_valid() {
            break;
        }
        let Ok(attr) = session_query::get_named_attmap(read, catalog_ref).await else {
            break;
        };
        if matches!(attr.get_type_str(), "SCOM" | "SPRF" | "SFIT" | "JOIN") {
            break;
        }
        let Some(next_ref) = attr.get_foreign_refno("CATR") else {
            break;
        };
        if !next_ref.is_valid() || next_ref == catalog_ref {
            break;
        }
        catalog_ref = next_ref;
    }
    catalog_ref
}

/// Session 版 get_cat_refno：设计件可能只有 SPRE，需 SPRE→SPCO.CATR；或直接 CATR/CAT。
async fn resolve_catalog_ref_from_session(
    read: &GenerationReadContext,
    desi_att: &NamedAttrMap,
) -> Option<RefnoEnum> {
    for key in ["CATR", "CAT"] {
        if let Some(catr) = desi_att.get_foreign_refno(key) {
            if catr.is_valid() && !catr.is_unset() {
                return Some(catr);
            }
        }
    }
    let spre = desi_att.get_foreign_refno("SPRE")?;
    if !spre.is_valid() || spre.is_unset() {
        return None;
    }
    let spco = session_query::get_named_attmap(read, spre).await.ok()?;
    let catr = spco.get_foreign_refno("CATR")?;
    (catr.is_valid() && !catr.is_unset()).then_some(catr)
}

async fn cata_context_from_session(
    read: &GenerationReadContext,
    desi_refno: RefnoEnum,
    desi_att: &NamedAttrMap,
    is_tubi: bool,
) -> anyhow::Result<CataContext> {
    let mut context = CataContext::default();
    context.is_tubi = is_tubi;
    if let Some(value) = desi_att.get_as_string("JUSL") {
        context.insert("JUSL".to_string(), value);
    }
    context.insert("DESI_REFNO".to_string(), desi_refno.to_string());
    for (index, value) in desi_att
        .get_f32_vec("DESP")
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        context.insert(format!("DESI{}", index + 1), value.to_string());
        context.insert(format!("DESP{}", index + 1), value.to_string());
    }
    for (index, value) in desi_att.get_ddesp().unwrap_or_default().iter().enumerate() {
        context.insert(format!("DDES{}", index + 1), value.to_string());
    }
    context.insert(
        "DDHEIGHT".to_string(),
        desi_att
            .get_as_string("HEIG")
            .unwrap_or_else(|| "0.0".into()),
    );
    context.insert(
        "DDANGLE".to_string(),
        desi_att
            .get_as_string("ANGL")
            .unwrap_or_else(|| "0.0".into()),
    );
    context.insert(
        "DDRADIUS".to_string(),
        desi_att
            .get_as_string("RADI")
            .unwrap_or_else(|| "0.0".into()),
    );
    for (name, value) in &desi_att.map {
        let name = name.to_ascii_uppercase();
        match value {
            NamedAttrValue::F32Type(value) => {
                context.insert(name, value.to_string());
            }
            NamedAttrValue::F32VecType(values) => {
                for (index, value) in values.iter().enumerate() {
                    context.insert(format!("{}{}", name, index + 1), value.to_string());
                }
            }
            _ => {}
        }
    }
    context.insert("RS_DES_REFNO".to_string(), desi_refno.to_string());

    let cat_refno = desi_att.get_foreign_refno("CATR");
    if let Some(cat_refno) = cat_refno {
        let cata_attmap = session_query::get_named_attmap(read, cat_refno).await?;
        context.insert("RS_CATR_REFNO".to_string(), cat_refno.to_string());
        for (index, value) in cata_attmap
            .get_f32_vec("PARA")
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            context.insert(format!("CPAR{}", index + 1), value.to_string());
            context.insert(format!("PARA{}", index + 1), value.to_string());
            context.insert(format!("PARAM{}", index + 1), value.to_string());
            context.insert(format!("IPARA{}", index + 1), "0".to_string());
            context.insert(format!("IPAR{}", index + 1), "0".to_string());
        }

        let mut owner_ref = desi_att.get_owner();
        let mut visited_owners = std::collections::BTreeSet::new();
        anyhow::ensure!(
            visited_owners.insert(owner_ref),
            "CATA context owner cycle at {owner_ref}"
        );
        let mut owner_att = session_query::get_named_attmap(read, owner_ref).await?;
        while !owner_att.contains_key("GTYP")
            && owner_att.get_refno().is_some()
            && owner_att.get_type_str() != "ZONE"
        {
            owner_ref = owner_att.get_owner();
            anyhow::ensure!(
                visited_owners.insert(owner_ref),
                "CATA context owner cycle at {owner_ref}"
            );
            owner_att = session_query::get_named_attmap(read, owner_ref).await?;
        }
        if let Some(dtre_refno) = cata_attmap.get_foreign_refno("DTRE") {
            let children = session_query::get_children(read, dtre_refno);
            for child in session_query::get_named_attmaps(read, &children)
                .await?
                .into_values()
            {
                if let Some(key) = child.get_as_string("DKEY") {
                    let key = format!("RPRO_{key}");
                    context.insert(key.clone(), child.get_as_string("PPRO").unwrap_or_default());
                    context.insert(
                        format!("{key}_default_expr"),
                        child.get_as_string("DPRO").unwrap_or_default(),
                    );
                    context.insert(
                        format!("{key}_default_type"),
                        child.get_as_string("PTYP").unwrap_or_default(),
                    );
                }
            }
        }
        for (index, value) in owner_att
            .get_f32_vec("DESP")
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            context.insert(format!("ODES{}", index + 1), value.to_string());
        }
        if let Some(owner_cat_ref) = owner_att.get_foreign_refno("CATR")
            && let Ok(owner_cat) = session_query::get_named_attmap(read, owner_cat_ref).await
        {
            for (index, value) in owner_cat
                .get_f32_vec("PARA")
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                context.insert(format!("OPAR{}", index + 1), value.to_string());
            }
        }
        if let Some(cref) = desi_att.get_foreign_refno("CREF")
            && let Ok(cref_att) = session_query::get_named_attmap(read, cref).await
        {
            for (index, value) in cref_att
                .get_f32_vec("DESP")
                .unwrap_or_default()
                .iter()
                .enumerate()
            {
                context.insert(format!("ADES{}", index + 1), value.to_string());
            }
            if let Some(cref_cat_ref) = cref_att.get_foreign_refno("CATR")
                && let Ok(cref_cat) = session_query::get_named_attmap(read, cref_cat_ref).await
            {
                for (index, value) in cref_cat
                    .get_f32_vec("PARA")
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                {
                    context.insert(format!("APAR{}", index + 1), value.to_string());
                }
            }
        }
    }
    Ok(context)
}

///求解design component
pub async fn resolve_desi_comp(
    desi_refno: RefnoEnum,
    tubi_scom: Option<RefnoEnum>,
    desi_att_opt: Option<&NamedAttrMap>,
) -> anyhow::Result<CateGeomsInfo> {
    resolve_desi_comp_inner(None, desi_refno, tubi_scom, desi_att_opt).await
}

pub async fn resolve_desi_comp_with_session(
    read: &GenerationReadContext,
    desi_refno: RefnoEnum,
    tubi_scom: Option<RefnoEnum>,
    desi_att_opt: Option<&NamedAttrMap>,
) -> anyhow::Result<CateGeomsInfo> {
    resolve_desi_comp_inner(Some(read), desi_refno, tubi_scom, desi_att_opt).await
}

async fn resolve_desi_comp_inner(
    read: Option<&GenerationReadContext>,
    desi_refno: RefnoEnum,
    tubi_scom: Option<RefnoEnum>,
    desi_att_opt: Option<&NamedAttrMap>,
) -> anyhow::Result<CateGeomsInfo> {
    let trace_resolve = should_trace_resolve_desi(desi_refno);
    let t_total = Instant::now();
    let t_desi_att = Instant::now();
    let owned_att;
    let desi_att = if let Some(att) = desi_att_opt {
        att
    } else {
        owned_att = match read {
            Some(read) => session_query::get_named_attmap(read, desi_refno).await?,
            None => aios_core::get_named_attmap(desi_refno).await?,
        };
        &owned_att
    };
    let desi_att_fetch_time = t_desi_att.elapsed().as_millis();
    let initial_tubi_scom = tubi_scom;
    let mut is_tubi = initial_tubi_scom.is_some();

    let t_scom_ref = Instant::now();
    let mut scom_ref = if let Some(scom) = tubi_scom {
        scom
    } else {
        let scom = match read {
            Some(read) => resolve_catalog_ref_from_session(read, desi_att).await,
            None => aios_core::get_cat_refno(desi_refno)
                .await?
                .or_else(|| desi_att.get_foreign_refno("CATR")),
        }
        .ok_or(anyhow::anyhow!(format!(
            "CAT引用不存在: {}",
            desi_refno.to_string()
        )))?;
        scom
    };
    let normalized_scom_ref = match read {
        Some(read) => normalize_catalog_scom_ref_from_session(read, scom_ref).await,
        None => normalize_catalog_scom_ref(scom_ref).await,
    };
    if normalized_scom_ref != scom_ref {
        if initial_tubi_scom == Some(desi_refno) {
            is_tubi = false;
        }
        debug_model_trace!(
            "catalog ref normalized: design_refno={:?}, from={:?}, to={:?}, is_tubi={}",
            desi_refno,
            scom_ref,
            normalized_scom_ref,
            is_tubi
        );
        scom_ref = normalized_scom_ref;
    }
    let scom_ref_time = t_scom_ref.elapsed().as_millis();
    debug_model_trace!("scom_ref: {:?}", &scom_ref);
    let t_scom_info = Instant::now();
    let scom_info = match read {
        Some(read) => get_or_create_scom_info_from_session(read, scom_ref).await?,
        None => get_or_create_scom_info(scom_ref).await?,
    };
    let scom_info_time = t_scom_info.elapsed().as_millis();
    debug_model_trace!("scom_info: {:?}", &scom_info);
    let t_context = Instant::now();
    let mut context = match read {
        Some(read) => cata_context_from_session(read, desi_refno, desi_att, is_tubi).await?,
        None => {
            aios_core::rs_surreal::resolve::get_or_create_cata_context(desi_refno, is_tubi).await?
        }
    };
    let context_time = t_context.elapsed().as_millis();

    let t_bind_params = Instant::now();
    // 🔍 调试：打印 DESI 的 DESP 数据（复用已有的 desi_att，避免重复 I/O）
    {
        if let Some(desp) = desi_att.get_f32_vec("DESP") {
            debug_model_trace!("   ✅ DESP array: {:?}", desp);
            if let Some(unipar) = desi_att.get_i32_vec("UNIPAR") {
                debug_model_trace!("   UNIPAR array: {:?}", unipar);

                use aios_core::consts::WORD_HASH;
                use aios_core::tool::db_tool::db1_dehash;

                for (i, (&value, &utype)) in desp.iter().zip(unipar.iter()).enumerate() {
                    if utype == WORD_HASH as i32 {
                        let word = db1_dehash(value as u32);
                        debug_model_trace!(
                            "      DESP[{}] = {} ⚠️  类型=WORD, dehash='{}'",
                            i,
                            value,
                            word
                        );
                    } else {
                        debug_model_trace!("      DESP[{}] = {} ✅ 类型=数值", i, value);
                    }
                }
            }
        } else {
            debug_model_trace!("   ⚠️  DESI 没有 DESP 属性");
        }
    }

    // 添加 SCOM 的 PARA 数组到 context 中
    // PARA 字符串格式: " 100.000 100.000 534980.000 ..."
    // 需要解析为: "PARAM 0" = "100.0", "PARAM 1" = "100.0", ...
    // 注意：表达式解析器会将 "PARAM" 截断为 "PARA"（去掉末尾的 "M"）
    // 所以需要同时添加 "PARA0", "PARAM0", "PARAM 0" 等多个版本
    let para_str = &scom_info.params;
    let para_values: Vec<&str> = para_str.split_whitespace().collect();

    // 🔍 调试输出：打印 PARA 字符串和解析结果
    debug_model_trace!(
        "🔍 [SCOM PARA] desi_refno={:?}, scom_refno={:?}",
        desi_refno,
        scom_ref
    );
    debug_model_trace!("   PARA string: '{}'", para_str);
    debug_model_trace!("   Parsed values: {:?}", para_values);

    // 从已有的 scom_info.attr_map 获取 UNIPAR（避免重复 I/O）
    let scom_attmap = &scom_info.attr_map;
    let unipar_vec = {
        if let Some(raw_para) = scom_attmap.get_as_string("PARA") {
            debug_model_trace!("   🔍 SurrealDB 原始 PARA: '{}'", raw_para);
        }
        debug_model_trace!("   🔍 SCOM name: {:?}", scom_attmap.get_as_string("NAME"));
        debug_model_trace!("   🔍 SCOM noun: {:?}", scom_attmap.get_type_str());

        if let Some(unipar) = scom_attmap.get_i32_vec("UNIPAR") {
            debug_model_trace!("   🔍 UNIPAR (参数类型): {:?}", unipar);

            use aios_core::tool::db_tool::db1_dehash;

            for (i, (value, &utype)) in para_values.iter().zip(unipar.iter()).enumerate() {
                if utype == WORD_HASH as i32 {
                    if let Ok(num_value) = value.parse::<f32>() {
                        let word = db1_dehash(num_value as u32);
                        debug_model_trace!(
                            "      PARA[{}] = {} ⚠️  类型=WORD, dehash='{}'",
                            i,
                            value,
                            word
                        );
                    } else {
                        debug_model_trace!(
                            "      PARA[{}] = {} ⚠️  类型=WORD (无法解析为数字)",
                            i,
                            value
                        );
                    }
                } else {
                    debug_model_trace!("      PARA[{}] = {} ✅ 类型=数值 (几何尺寸)", i, value);
                }
            }
            Some(unipar)
        } else {
            debug_model_trace!("   ⚠️  SCOM 没有 UNIPAR 属性");
            None
        }
    };

    // ⚠️  重要修复：
    // 1. 表达式中的 "PARAM" 实际上是指 SCOM 的 PARA 参数
    // 2. "PARAM 2" 会被转换为 "PARA2"（索引从 1 开始）
    // 3. 需要将 SCOM 的 PARA 添加为 "PARA1", "PARA2", ... 等键名
    // 4. 但是需要过滤掉 WORD 类型的参数（UNIPAR[i] = 623723）

    // 添加 SCOM 的 PARA 为 PARA1, PARA2, ...（索引从 1 开始）
    for (i, value) in para_values.iter().enumerate() {
        let is_word_type = unipar_vec
            .as_ref()
            .and_then(|unipar| unipar.get(i))
            .map(|&u| u == WORD_HASH as i32)
            .unwrap_or(false);

        if is_word_type {
            // WORD 类型的参数不应该用于几何计算，使用默认值 0.0
            debug_model_trace!(
                "   ⚠️  PARA[{}] 是 WORD 类型，PARA{} 使用默认值 0.0",
                i,
                i + 1
            );
            context.insert(format!("PARA{}", i + 1), "0.0".to_string());
        } else {
            context.insert(format!("PARA{}", i + 1), value.to_string());
        }

        // 同时添加 CPAR（Catalogue Parameter）
        context.insert(format!("CPAR{}", i + 1), value.to_string());
    }

    // IPARAM（保温层参数）：根据 context.with_insulation 开关决定是否代入实际值。
    // - false（默认）：IPARAM 全部为 0，生成物理几何模型
    // - true：IPARAM 使用实际保温层厚度（来自 ISPE→SPCO→CATR.PARA），生成含保温层的模型
    const DEFAULT_IPARAM_COUNT: usize = 32;
    if !context.with_insulation {
        // 性能/稳定性：默认物理模型不需要从 DB 读取保温层参数。
        // 同时也规避某些环境下 SurrealDB 自定义函数 fn::get_ipara 因缺失字段返回 NONE 导致的 array::filter 报错。
        for idx1 in 1..=DEFAULT_IPARAM_COUNT {
            insert_iparam_kv(&mut context, idx1, "0");
        }
    } else {
        let query_result = match read {
            Some(read) => query_iparam_from_session(read, desi_att).await,
            None => query_iparam_from_desi(desi_refno).await,
        };
        match query_result {
            Ok(mut iparams) => {
                debug_model_debug!(
                    "IPARAM query result: {:?}, with_insulation={}",
                    iparams,
                    context.with_insulation
                );

                if iparams.len() < DEFAULT_IPARAM_COUNT {
                    iparams.resize(DEFAULT_IPARAM_COUNT, 0.0);
                }
                for (i, value) in iparams.iter().enumerate() {
                    insert_iparam_kv(&mut context, i + 1, &value.to_string());
                }
            }
            Err(e) => {
                // 保温层场景下若 DB 查询失败，降级为 0，避免表达式缺键导致整体失败。
                crate::smart_debug_error!("Failed to query IPARAM (fallback to 0): {}", e);
                for idx1 in 1..=DEFAULT_IPARAM_COUNT {
                    insert_iparam_kv(&mut context, idx1, "0");
                }
            }
        }
    }
    let bind_params_time = t_bind_params.elapsed().as_millis();

    crate::smart_debug_model_debug!("=== DEBUG: CataContext for {} ===", desi_refno.to_string());
    crate::smart_debug_model_debug!("Context entries count: {}", context.context.len());
    crate::smart_debug_model_debug!("PARA string: {}", para_str);
    crate::smart_debug_model_debug!("Parsed {} PARAM values", para_values.len());
    // 打印所有 PARAM 相关的键值对
    if aios_core::is_debug_model_enabled() {
        for entry in context.context.iter() {
            let key = entry.key();
            let value = entry.value();
            if key.contains("PARAM") || key.contains("PARA") || key.contains("IPARAM") {
                // debug_model_debug!("  {} = {}", key, value);
            }
        }
    }
    // debug_model_debug!("=== END Context ===");

    // 🔍 表达式预验证：在调用 resolve_cata_comp 前检查所有表达式的语法
    // 这有助于快速定位元件库中的表达式错误
    let t_validate = Instant::now();
    if aios_core::is_debug_model_enabled() {
        let scom_name = scom_info
            .attr_map
            .get_as_string("NAME")
            .unwrap_or_else(|| "未知".to_string());
        validate_scom_expressions(desi_refno, scom_ref, &scom_name, &scom_info);
    }
    let validate_expr_time = t_validate.elapsed().as_millis();

    let context_entry_count = context.context.len();
    let t_resolve_comp = Instant::now();
    let geom_info = resolve_cata_comp(&desi_att, &scom_info, Some(context));
    let resolve_comp_time = t_resolve_comp.elapsed().as_millis();
    debug_model_trace!("geom_info: {:?}", &geom_info);
    if trace_resolve {
        println!(
            "    [resolve trace] refno={} total={}ms att={}ms scom_ref={}ms scom_info={}ms context={}ms bind_params={}ms validate={}ms resolve_comp={}ms context_entries={} para={} axis={} gm={} ngm={} is_tubi={}",
            desi_refno,
            t_total.elapsed().as_millis(),
            desi_att_fetch_time,
            scom_ref_time,
            scom_info_time,
            context_time,
            bind_params_time,
            validate_expr_time,
            resolve_comp_time,
            context_entry_count,
            para_values.len(),
            scom_info.axis_params.len(),
            scom_info.gm_params.len(),
            scom_info.ngm_params.len(),
            is_tubi
        );
    }

    match geom_info {
        Ok(info) => Ok(info),
        Err(e) => {
            use crate::fast_model::ModelErrorKind;
            crate::model_error!(
                code = "E-EXPR-001",
                kind = ModelErrorKind::InvalidGeometry,
                stage = "resolve_cata_comp",
                refno = desi_refno,
                desc = "表达式计算失败",
                "design_refno={}, scom_ref={}, err={}",
                desi_refno,
                scom_ref,
                e
            );
            Err(anyhow!("resolve_cata_comp 表达式计算失败: {}", e))
        }
    }
}

/// 验证 SCOM（元件库）中所有几何体的表达式
/// 在 resolve_cata_comp 调用前进行预验证，便于快速定位数据问题
fn validate_scom_expressions(
    desi_refno: RefnoEnum,
    scom_refno: RefnoEnum,
    scom_name: &str,
    scom_info: &ScomInfo,
) {
    let mut all_errors = Vec::new();

    // 验证正向几何体 (gm_params)
    for gm in &scom_info.gm_params {
        let errors = validate_gm_param_expressions(gm);
        all_errors.extend(errors);
    }

    // 验证负向几何体 (ngm_params)
    for gm in &scom_info.ngm_params {
        let errors = validate_gm_param_expressions(gm);
        all_errors.extend(errors);
    }

    // 如果有错误，记录详细的错误信息
    if !all_errors.is_empty() {
        use crate::fast_model::ModelErrorKind;

        for error in &all_errors {
            crate::model_error!(
                code = "E-EXPR-002",
                kind = ModelErrorKind::InvalidGeometry,
                stage = "expression_prevalidation",
                refno = desi_refno,
                desc = "元件库表达式语法错误",
                "design_refno={}, scom_refno={}, scom_name='{}', gm_refno={}, gm_type={}, attr={}, expr='{}', error={}",
                desi_refno,
                scom_refno,
                scom_name,
                error.gm_refno,
                error.gm_type,
                error.attr_name,
                error.expression,
                error.message
            );
        }

        // 这些表达式错误可能非常多，stdout/stderr 会显著拖慢 profile。
        // 需要时可通过以下开关输出：
        // - `--debug-model`（调试单个 refno）或
        // - 环境变量 `AIOS_EXPR_PREVALIDATION_STDERR=1|true`
        // 同时支持将详细错误写入 tracing 日志：
        // - 环境变量 `AIOS_EXPR_PREVALIDATION_LOG=1|true`
        let stderr_enabled = std::env::var("AIOS_EXPR_PREVALIDATION_STDERR")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let log_enabled = std::env::var("AIOS_EXPR_PREVALIDATION_LOG")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let debug_enabled = aios_core::is_debug_model_enabled();

        if stderr_enabled || debug_enabled {
            eprintln!(
                "⚠️  [表达式预验证] design={}, scom={}({}): 发现 {} 个表达式错误",
                desi_refno,
                scom_refno,
                scom_name,
                all_errors.len()
            );
            for error in &all_errors {
                eprintln!("   - {}", error);
            }
        }

        if log_enabled || debug_enabled {
            tracing::warn!(
                design_refno = %desi_refno,
                scom_refno = %scom_refno,
                scom_name = scom_name,
                error_cnt = all_errors.len(),
                "expression prevalidation: invalid expressions found"
            );
            // 只有在明确打开开关时才逐条写日志（避免日志爆炸）。
            if log_enabled {
                for error in &all_errors {
                    tracing::warn!(
                        design_refno = %desi_refno,
                        scom_refno = %scom_refno,
                        scom_name = scom_name,
                        gm_refno = error.gm_refno.as_str(),
                        gm_type = error.gm_type.as_str(),
                        attr = error.attr_name.as_str(),
                        expr = error.expression.as_str(),
                        msg = error.message.as_str(),
                        "expression prevalidation error"
                    );
                }
            }
        }
    }
}

/// 验证单个 GmParam 中的所有表达式
fn validate_gm_param_expressions(
    gm: &GmParam,
) -> Vec<crate::expression_fix::ExpressionValidationError> {
    let gm_refno = gm.refno.to_string();
    let gm_type = &gm.gm_type;

    // 收集所有需要验证的表达式
    let mut expressions: Vec<(&str, &str)> = vec![
        ("prad", &gm.prad),
        ("pang", &gm.pang),
        ("pwid", &gm.pwid),
        ("phei", &gm.phei),
        ("offset", &gm.offset),
        ("drad", &gm.drad),
        ("dwid", &gm.dwid),
    ];

    // 添加数组类型的表达式
    for (i, expr) in gm.diameters.iter().enumerate() {
        // 使用临时 String 存储属性名，避免生命周期问题
        expressions.push((
            Box::leak(format!("diameters[{}]", i).into_boxed_str()),
            expr.as_str(),
        ));
    }
    for (i, expr) in gm.distances.iter().enumerate() {
        expressions.push((
            Box::leak(format!("distances[{}]", i).into_boxed_str()),
            expr.as_str(),
        ));
    }
    for (i, expr) in gm.shears.iter().enumerate() {
        expressions.push((
            Box::leak(format!("shears[{}]", i).into_boxed_str()),
            expr.as_str(),
        ));
    }
    for (i, expr) in gm.lengths.iter().enumerate() {
        expressions.push((
            Box::leak(format!("lengths[{}]", i).into_boxed_str()),
            expr.as_str(),
        ));
    }
    for (i, expr) in gm.xyz.iter().enumerate() {
        expressions.push((
            Box::leak(format!("xyz[{}]", i).into_boxed_str()),
            expr.as_str(),
        ));
    }
    for (i, expr) in gm.frads.iter().enumerate() {
        expressions.push((
            Box::leak(format!("frads[{}]", i).into_boxed_str()),
            expr.as_str(),
        ));
    }
    for (i, vert) in gm.verts.iter().enumerate() {
        expressions.push((
            Box::leak(format!("verts[{}].x", i).into_boxed_str()),
            vert[0].as_str(),
        ));
        expressions.push((
            Box::leak(format!("verts[{}].y", i).into_boxed_str()),
            vert[1].as_str(),
        ));
        expressions.push((
            Box::leak(format!("verts[{}].z", i).into_boxed_str()),
            vert[2].as_str(),
        ));
    }
    for (i, dxy) in gm.dxy.iter().enumerate() {
        expressions.push((
            Box::leak(format!("dxy[{}].x", i).into_boxed_str()),
            dxy[0].as_str(),
        ));
        expressions.push((
            Box::leak(format!("dxy[{}].y", i).into_boxed_str()),
            dxy[1].as_str(),
        ));
    }
    for (i, axis) in gm.paxises.iter().enumerate() {
        expressions.push((
            Box::leak(format!("paxises[{}]", i).into_boxed_str()),
            axis.as_str(),
        ));
    }

    ExpressionFixer::validate_gm_param_expressions(&gm_refno, gm_type, &expressions)
}
