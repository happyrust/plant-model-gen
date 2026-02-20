use crate::fast_model::query_gm_params;
use crate::fast_model::{debug_model, debug_model_debug, debug_model_trace};
use crate::expression_fix::ExpressionFixer;
use aios_core::SurrealQueryExt;
use aios_core::consts::WORD_HASH;
use aios_core::expression::query_cata::{query_axis_params, resolve_cata_comp};
use aios_core::expression::resolve::{SCOM_INFO_MAP, resolve_axis_param};
use aios_core::parsed_data::{CateAxisParam, CateGeomsInfo};
use aios_core::pdms_data::{PlinParam, ScomInfo, GmParam};
use aios_core::{CataContext, NamedAttrMap, RefU64, RefnoEnum, SUL_DB};
use anyhow::anyhow;
use std::collections::{BTreeMap, HashMap};

fn normalize_gm_param_expressions_in_place(gm: &mut GmParam) {
    // 仅做“去掉 ATTRIB :NAME 中的冒号”这种低风险规整，避免 aios_core 表达式解析器直接拒绝。
    // 不做更激进的重写（例如移除 ATTRIB 或把 [n] 展平），以降低行为回归风险。
    gm.prad = ExpressionFixer::normalize_attrib_colon(&gm.prad);
    gm.pang = ExpressionFixer::normalize_attrib_colon(&gm.pang);
    gm.pwid = ExpressionFixer::normalize_attrib_colon(&gm.pwid);
    gm.phei = ExpressionFixer::normalize_attrib_colon(&gm.phei);
    gm.offset = ExpressionFixer::normalize_attrib_colon(&gm.offset);
    gm.drad = ExpressionFixer::normalize_attrib_colon(&gm.drad);
    gm.dwid = ExpressionFixer::normalize_attrib_colon(&gm.dwid);

    for expr in gm.diameters.iter_mut() {
        *expr = ExpressionFixer::normalize_attrib_colon(expr);
    }
    for expr in gm.distances.iter_mut() {
        *expr = ExpressionFixer::normalize_attrib_colon(expr);
    }
}

/// 查询 DESI 元素的 IPARAM 数据
/// 使用 SurrealDB 的 fn::get_ipara 函数
async fn query_iparam_from_desi(desi_refno: RefnoEnum) -> anyhow::Result<Vec<f32>> {
    let sql = format!("return fn::get_ipara({})", desi_refno.to_pe_key());
    let result: Vec<f32> = SUL_DB.query_take(&sql, 0).await?;

    Ok(result)
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
        let attr_map = aios_core::get_named_attmap(cata_refno).await?;
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

///求解design component
pub async fn resolve_desi_comp(
    desi_refno: RefnoEnum,
    mut tubi_scom: Option<RefnoEnum>,
    desi_att_opt: Option<&NamedAttrMap>,
) -> anyhow::Result<CateGeomsInfo> {
    let owned_att;
    let desi_att = if let Some(att) = desi_att_opt {
        att
    } else {
        owned_att = aios_core::get_named_attmap(desi_refno).await?;
        &owned_att
    };
    let is_tubi = tubi_scom.is_some();

    let scom_ref = if let Some(scom) = tubi_scom {
        scom
    } else {
        let scom = aios_core::get_cat_refno(desi_refno)
            .await?
            .ok_or(anyhow::anyhow!(format!(
                "CAT引用不存在: {}",
                desi_refno.to_string()
            )))?;
        scom
    };
    debug_model_trace!("scom_ref: {:?}", &scom_ref);
    let scom_info = get_or_create_scom_info(scom_ref).await?;
    debug_model_trace!("scom_info: {:?}", &scom_info);
    let mut context = aios_core::create_cata_context_with_att(
        desi_refno, desi_att, scom_ref, &scom_info.attr_map, is_tubi,
    ).await.unwrap();

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
                            i, value, word
                        );
                    } else {
                        debug_model_trace!(
                            "      PARA[{}] = {} ⚠️  类型=WORD (无法解析为数字)",
                            i, value
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
        match query_iparam_from_desi(desi_refno).await {
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
    if aios_core::is_debug_model_enabled() {
        let scom_name = scom_info.attr_map.get_as_string("NAME").unwrap_or_else(|| "未知".to_string());
        validate_scom_expressions(desi_refno, scom_ref, &scom_name, &scom_info);
    }

    let geom_info = resolve_cata_comp(&desi_att, &scom_info, Some(context));
    debug_model_trace!("geom_info: {:?}", &geom_info);

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
fn validate_gm_param_expressions(gm: &GmParam) -> Vec<crate::expression_fix::ExpressionValidationError> {
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
        expressions.push((Box::leak(format!("diameters[{}]", i).into_boxed_str()), expr.as_str()));
    }
    for (i, expr) in gm.distances.iter().enumerate() {
        expressions.push((Box::leak(format!("distances[{}]", i).into_boxed_str()), expr.as_str()));
    }
    for (i, expr) in gm.shears.iter().enumerate() {
        expressions.push((Box::leak(format!("shears[{}]", i).into_boxed_str()), expr.as_str()));
    }
    for (i, expr) in gm.lengths.iter().enumerate() {
        expressions.push((Box::leak(format!("lengths[{}]", i).into_boxed_str()), expr.as_str()));
    }
    for (i, expr) in gm.xyz.iter().enumerate() {
        expressions.push((Box::leak(format!("xyz[{}]", i).into_boxed_str()), expr.as_str()));
    }
    for (i, expr) in gm.frads.iter().enumerate() {
        expressions.push((Box::leak(format!("frads[{}]", i).into_boxed_str()), expr.as_str()));
    }
    for (i, vert) in gm.verts.iter().enumerate() {
        expressions.push((Box::leak(format!("verts[{}].x", i).into_boxed_str()), vert[0].as_str()));
        expressions.push((Box::leak(format!("verts[{}].y", i).into_boxed_str()), vert[1].as_str()));
        expressions.push((Box::leak(format!("verts[{}].z", i).into_boxed_str()), vert[2].as_str()));
    }
    for (i, dxy) in gm.dxy.iter().enumerate() {
        expressions.push((Box::leak(format!("dxy[{}].x", i).into_boxed_str()), dxy[0].as_str()));
        expressions.push((Box::leak(format!("dxy[{}].y", i).into_boxed_str()), dxy[1].as_str()));
    }
    for (i, axis) in gm.paxises.iter().enumerate() {
        expressions.push((Box::leak(format!("paxises[{}]", i).into_boxed_str()), axis.as_str()));
    }

    ExpressionFixer::validate_gm_param_expressions(&gm_refno, gm_type, &expressions)
}
