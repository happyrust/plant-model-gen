//! v3 实例导出：精简 JSON 格式
//!
//! 参考 Parquet 导出的规范化数据模型，变换矩阵通过 hash 引用去重。
//!
//! 输出结构：
//! - 顶层 `transforms` 字典：hash → [16 f32] 列主序矩阵
//! - 顶层 `aabb` 字典：hash → [min_x, min_y, min_z, max_x, max_y, max_z]
//! - `bran_groups` / `equi_groups` / `ungrouped`：实例分组
//! - 每个实例通过 `trans_hash` / `geo_trans_hash` 引用变换矩阵
//!
//! 与 v2 的主要区别：
//! - 去掉 color_index / name_index / site_name_index / name / lod_mask / uniforms
//! - 矩阵去重存储（体积缩减约 40-60%）
//! - 单位转换和坐标旋转可通过 ExportTransformConfig 配置

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use aios_core::SurrealQueryExt;
use aios_core::options::DbOption;
use aios_core::pdms_types::RefnoEnum;
use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use glam::DMat4;
use serde::{Deserialize, Serialize};
use serde_json::json;
use surrealdb::types::SurrealValue;

use super::InstRelateRow;
use super::export_transform_config::ExportTransformConfig;
use crate::fast_model::unit_converter::UnitConverter;

// =============================================================================
// 公共返回类型
// =============================================================================

pub struct V3ExportStats {
    pub bran_group_count: usize,
    pub equi_group_count: usize,
    pub ungrouped_count: usize,
    pub total_component_instances: usize,
    pub total_tubing_instances: usize,
    pub transform_count: usize,
    pub aabb_count: usize,
    pub elapsed: std::time::Duration,
    pub output_filename: String,
}

// =============================================================================
// SurrealDB 查询结构体
// =============================================================================

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct TubiQueryRow {
    pub refno: RefnoEnum,
    pub index: Option<i64>,
    pub leave: RefnoEnum,
    pub world_aabb_hash: Option<String>,
    pub world_trans_hash: Option<String>,
    pub geo_hash: Option<String>,
    pub spec_value: Option<i64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct TransQueryRow {
    hash: String,
    d: serde_json::Value,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct AabbQueryRow {
    hash: String,
    d: Option<aios_core::types::PlantAabb>,
}

// =============================================================================
// 内部数据结构
// =============================================================================

struct OwnerGroup {
    owner_type: String,
    children: Vec<ChildInfo>,
}

struct ChildInfo {
    refno: RefnoEnum,
    noun: String,
    spec_value: i64,
}

struct TubiInfo {
    leave_refno: RefnoEnum,
    owner_refno: RefnoEnum,
    order: usize,
    geo_hash: String,
    trans_hash: Option<String>,
    aabb_hash: Option<String>,
    spec_value: i64,
}

struct UngroupedInfo {
    refno: RefnoEnum,
    noun: String,
}

// =============================================================================
// 主导出函数
// =============================================================================

pub async fn export_dbnum_instances_v3(
    dbnum: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    transform_config: ExportTransformConfig,
    root_refno: Option<RefnoEnum>,
) -> Result<V3ExportStats> {
    let start_time = std::time::Instant::now();
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    if verbose {
        println!("🚀 [v3] 开始导出 dbnum={} 的实例数据", dbnum);
        if transform_config.needs_unit_conversion() {
            println!(
                "   单位转换: {} → {}",
                transform_config.source_unit.name(),
                transform_config.target_unit.name()
            );
        }
        if transform_config.apply_rotation {
            println!("   坐标旋转: Z-up → Y-up");
        }
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    // =========================================================================
    // 1. 查询 inst_relate
    // =========================================================================
    // 确保 db_meta 已加载（root_refno 模式需要通过 db_meta 映射 refno→dbnum）
    {
        use crate::data_interface::db_meta;
        let _ = db_meta().ensure_loaded();
    }

    let inst_rows = if let Some(root) = root_refno {
        use crate::fast_model::query_compat::query_deep_visible_inst_refnos;
        if verbose {
            println!("🔍 查询 {} 的可见实例节点...", root);
        }
        let sub_refnos = query_deep_visible_inst_refnos(root).await?;
        if verbose {
            println!("   ✅ 子树 refno 数量: {}", sub_refnos.len());
        }
        super::query_inst_relate_batch(&sub_refnos, false, verbose).await?
    } else {
        query_inst_relate_by_dbnum(dbnum, verbose).await?
    };

    if verbose {
        println!("   ✅ inst_relate 记录: {} 条", inst_rows.len());
    }

    // =========================================================================
    // 2. 按 owner 分组
    // =========================================================================
    let mut bran_groups: HashMap<RefnoEnum, OwnerGroup> = HashMap::new();
    let mut equi_groups: HashMap<RefnoEnum, OwnerGroup> = HashMap::new();
    let mut ungrouped: Vec<UngroupedInfo> = Vec::new();
    let mut in_refnos: Vec<RefnoEnum> = Vec::new();
    let mut in_refno_set: HashSet<RefnoEnum> = HashSet::new();

    for row in &inst_rows {
        let owner_type = row
            .owner_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let spec_value = row.spec_value.unwrap_or(0);

        if in_refno_set.insert(row.refno) {
            in_refnos.push(row.refno);
        }

        match owner_type.as_str() {
            "BRAN" | "HANG" => {
                if let Some(owner) = row.owner_refno {
                    bran_groups
                        .entry(owner)
                        .or_insert_with(|| OwnerGroup {
                            owner_type: owner_type.clone(),
                            children: Vec::new(),
                        })
                        .children
                        .push(ChildInfo {
                            refno: row.refno,
                            noun: row.noun.clone().unwrap_or_default(),
                            spec_value,
                        });
                } else {
                    ungrouped.push(UngroupedInfo {
                        refno: row.refno,
                        noun: row.noun.clone().unwrap_or_default(),
                    });
                }
            }
            "EQUI" => {
                if let Some(owner) = row.owner_refno {
                    equi_groups
                        .entry(owner)
                        .or_insert_with(|| OwnerGroup {
                            owner_type: "EQUI".to_string(),
                            children: Vec::new(),
                        })
                        .children
                        .push(ChildInfo {
                            refno: row.refno,
                            noun: row.noun.clone().unwrap_or_default(),
                            spec_value,
                        });
                } else {
                    ungrouped.push(UngroupedInfo {
                        refno: row.refno,
                        noun: row.noun.clone().unwrap_or_default(),
                    });
                }
            }
            _ => {
                ungrouped.push(UngroupedInfo {
                    refno: row.refno,
                    noun: row.noun.clone().unwrap_or_default(),
                });
            }
        }
    }

    // =========================================================================
    // 3. 查询几何体实例 hash
    // =========================================================================
    if verbose {
        println!("🔍 查询 {} 个 refno 的几何体实例 hash...", in_refnos.len());
    }
    let mut export_inst_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();
    if !in_refnos.is_empty() {
        match query_export_insts(&in_refnos, true).await {
            Ok(export_insts) => {
                for inst in export_insts {
                    export_inst_map.insert(inst.refno, inst);
                }
                if verbose {
                    println!(
                        "   ✅ 有几何体的 refno: {}/{}",
                        export_inst_map.len(),
                        in_refnos.len()
                    );
                }
            }
            Err(e) => {
                if verbose {
                    println!("   ⚠️ 几何体实例查询失败: {:?}", e);
                }
            }
        }
    }

    // =========================================================================
    // 4. 查询 tubi_relate
    // =========================================================================
    let bran_owner_refnos: Vec<RefnoEnum> = bran_groups.keys().copied().collect();
    if verbose {
        println!(
            "🔍 查询 {} 个 BRAN/HANG owner 的 tubi_relate...",
            bran_owner_refnos.len()
        );
    }
    let tubings_map = query_tubi_relate(&bran_owner_refnos, verbose).await?;

    // =========================================================================
    // 5. 收集所有唯一 trans_hash 和 aabb_hash
    // =========================================================================
    let mut trans_hashes: HashSet<String> = HashSet::new();
    let mut aabb_hashes: HashSet<String> = HashSet::new();

    for export_inst in export_inst_map.values() {
        if let Some(ref h) = export_inst.world_trans_hash {
            if !h.is_empty() {
                trans_hashes.insert(h.clone());
            }
        }
        if let Some(ref h) = export_inst.world_aabb_hash {
            if !h.is_empty() {
                aabb_hashes.insert(h.clone());
            }
        }
        for inst in &export_inst.insts {
            if let Some(ref th) = inst.trans_hash {
                if !th.is_empty() {
                    trans_hashes.insert(th.clone());
                }
            }
        }
    }
    for tubis in tubings_map.values() {
        for tubi in tubis {
            if let Some(ref h) = tubi.world_trans_hash {
                if !h.is_empty() {
                    trans_hashes.insert(h.clone());
                }
            }
            if let Some(ref h) = tubi.world_aabb_hash {
                if !h.is_empty() {
                    aabb_hashes.insert(h.clone());
                }
            }
        }
    }

    // =========================================================================
    // 6. 批量查询 trans 和 aabb 实际数据
    // =========================================================================
    if verbose {
        println!(
            "🔍 查询 {} 个 trans, {} 个 aabb...",
            trans_hashes.len(),
            aabb_hashes.len()
        );
    }

    let unit_converter =
        UnitConverter::new(transform_config.source_unit, transform_config.target_unit);

    let (trans_map, aabb_map) = tokio::join!(
        resolve_trans_to_matrices(
            &trans_hashes,
            &unit_converter,
            transform_config.apply_rotation,
            verbose
        ),
        resolve_aabb(&aabb_hashes, &unit_converter, verbose),
    );
    let trans_map = trans_map?;
    let aabb_map = aabb_map?;

    if verbose {
        println!(
            "   ✅ trans 命中: {}, aabb 命中: {}",
            trans_map.len(),
            aabb_map.len()
        );
    }

    // =========================================================================
    // 7. 构建 JSON
    // =========================================================================
    let mut total_component_instances: usize = 0;
    let mut total_tubing_instances: usize = 0;

    // --- transforms 字典 ---
    let transforms_json: serde_json::Map<String, serde_json::Value> = trans_map
        .iter()
        .map(|(hash, cols)| (hash.clone(), json!(cols)))
        .collect();

    // --- aabb 字典 ---
    let aabb_json: serde_json::Map<String, serde_json::Value> = aabb_map
        .iter()
        .map(|(hash, vals)| (hash.clone(), json!(vals)))
        .collect();

    // --- bran_groups ---
    let mut bran_groups_json: Vec<serde_json::Value> = Vec::new();
    let mut bran_keys: Vec<RefnoEnum> = bran_groups.keys().copied().collect();
    bran_keys.sort();

    for owner_refno in &bran_keys {
        let group = &bran_groups[owner_refno];
        let mut children_json: Vec<serde_json::Value> = Vec::new();

        for child in &group.children {
            let export_inst = match export_inst_map.get(&child.refno) {
                Some(ei) if !ei.insts.is_empty() => ei,
                _ => continue,
            };

            let mut geos_json: Vec<serde_json::Value> = Vec::new();
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                geos_json.push(json!({
                    "geo_hash": inst.geo_hash,
                    "geo_index": geo_idx,
                    "geo_trans_hash": inst.trans_hash.as_deref().unwrap_or("0"),
                    "unit_mesh": inst.unit_flag,
                }));
                total_component_instances += 1;
            }

            children_json.push(json!({
                "refno": child.refno.to_string(),
                "noun": child.noun,
                "owner_noun": group.owner_type,
                "trans_hash": export_inst.world_trans_hash.as_deref().unwrap_or(""),
                "aabb_hash": export_inst.world_aabb_hash.as_deref().unwrap_or(""),
                "spec_value": child.spec_value,
                "has_neg": export_inst.has_neg,
                "geos": geos_json,
            }));
        }

        // TUBI
        let mut tubings_json: Vec<serde_json::Value> = Vec::new();
        if let Some(tubis) = tubings_map.get(owner_refno) {
            for tubi in tubis {
                tubings_json.push(json!({
                    "refno": tubi.leave.to_string(),
                    "owner_refno": owner_refno.to_string(),
                    "order": tubi.index.unwrap_or(0),
                    "geo_hash": tubi.geo_hash.as_deref().unwrap_or(""),
                    "trans_hash": tubi.world_trans_hash.as_deref().unwrap_or(""),
                    "aabb_hash": tubi.world_aabb_hash.as_deref().unwrap_or(""),
                    "spec_value": tubi.spec_value.unwrap_or(0),
                }));
                total_tubing_instances += 1;
            }
        }

        bran_groups_json.push(json!({
            "refno": owner_refno.to_string(),
            "noun": group.owner_type,
            "children": children_json,
            "tubings": tubings_json,
        }));
    }

    // --- equi_groups ---
    let mut equi_groups_json: Vec<serde_json::Value> = Vec::new();
    let mut equi_keys: Vec<RefnoEnum> = equi_groups.keys().copied().collect();
    equi_keys.sort();

    for owner_refno in &equi_keys {
        let group = &equi_groups[owner_refno];
        let mut children_json: Vec<serde_json::Value> = Vec::new();

        for child in &group.children {
            let export_inst = match export_inst_map.get(&child.refno) {
                Some(ei) if !ei.insts.is_empty() => ei,
                _ => continue,
            };

            let mut geos_json: Vec<serde_json::Value> = Vec::new();
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                geos_json.push(json!({
                    "geo_hash": inst.geo_hash,
                    "geo_index": geo_idx,
                    "geo_trans_hash": inst.trans_hash.as_deref().unwrap_or("0"),
                    "unit_mesh": inst.unit_flag,
                }));
                total_component_instances += 1;
            }

            children_json.push(json!({
                "refno": child.refno.to_string(),
                "noun": child.noun,
                "owner_noun": "EQUI",
                "trans_hash": export_inst.world_trans_hash.as_deref().unwrap_or(""),
                "aabb_hash": export_inst.world_aabb_hash.as_deref().unwrap_or(""),
                "spec_value": child.spec_value,
                "has_neg": export_inst.has_neg,
                "geos": geos_json,
            }));
        }

        equi_groups_json.push(json!({
            "refno": owner_refno.to_string(),
            "noun": "EQUI",
            "children": children_json,
        }));
    }

    // --- ungrouped ---
    let mut ungrouped_json: Vec<serde_json::Value> = Vec::new();
    for item in &ungrouped {
        let export_inst = match export_inst_map.get(&item.refno) {
            Some(ei) if !ei.insts.is_empty() => ei,
            _ => continue,
        };

        let mut geos_json: Vec<serde_json::Value> = Vec::new();
        for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
            geos_json.push(json!({
                "geo_hash": inst.geo_hash,
                "geo_index": geo_idx,
                "geo_trans_hash": inst.trans_hash.as_deref().unwrap_or("0"),
                "unit_mesh": inst.unit_flag,
            }));
            total_component_instances += 1;
        }

        ungrouped_json.push(json!({
            "refno": item.refno.to_string(),
            "noun": item.noun,
            "owner_noun": "",
            "trans_hash": export_inst.world_trans_hash.as_deref().unwrap_or(""),
            "aabb_hash": export_inst.world_aabb_hash.as_deref().unwrap_or(""),
            "spec_value": 0,
            "has_neg": export_inst.has_neg,
            "geos": geos_json,
        }));
    }

    // =========================================================================
    // 8. 组装并写入文件
    // =========================================================================
    let output_filename = match &root_refno {
        Some(root) => {
            let slug = root.to_string().replace(['/', '\\'], "_").replace(' ', "_");
            format!("instances_v3_root_{slug}.json")
        }
        None => format!("instances_v3_{dbnum}.json"),
    };

    let root_json = json!({
        "version": 3,
        "format": "json",
        "generated_at": generated_at,
        "dbnum": dbnum,
        "export_transform": transform_config.to_manifest_json(),
        "transforms": transforms_json,
        "aabb": aabb_json,
        "bran_groups": bran_groups_json,
        "equi_groups": equi_groups_json,
        "ungrouped": ungrouped_json,
    });

    let output_path = output_dir.join(&output_filename);
    let json_str = serde_json::to_string_pretty(&root_json)?;
    std::fs::write(&output_path, &json_str)?;

    let elapsed = start_time.elapsed();

    if verbose {
        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);
        println!("\n📊 [v3] 导出统计:");
        println!("   - BRAN/HANG 分组: {}", bran_groups_json.len());
        println!("   - EQUI 分组: {}", equi_groups_json.len());
        println!("   - 未分组: {}", ungrouped_json.len());
        println!("   - 构件实例: {}", total_component_instances);
        println!("   - TUBI 实例: {}", total_tubing_instances);
        println!("   - transforms 条目: {}", transforms_json.len());
        println!("   - aabb 条目: {}", aabb_json.len());
        println!("   - 文件大小: {:.2} MB", file_size as f64 / 1_048_576.0);
        println!("   - 耗时: {:.2}s", elapsed.as_secs_f64());
        println!("   ✅ 写入: {}", output_path.display());
    }

    Ok(V3ExportStats {
        bran_group_count: bran_groups_json.len(),
        equi_group_count: equi_groups_json.len(),
        ungrouped_count: ungrouped_json.len(),
        total_component_instances,
        total_tubing_instances,
        transform_count: transforms_json.len(),
        aabb_count: aabb_json.len(),
        elapsed,
        output_filename,
    })
}

// =============================================================================
// SurrealDB 查询函数
// =============================================================================

async fn query_inst_relate_by_dbnum(dbnum: u32, verbose: bool) -> Result<Vec<InstRelateRow>> {
    if verbose {
        println!(
            "🔍 扫描 inst_relate（索引路径: WHERE dbnum = {}）...",
            dbnum
        );
    }

    let sql = r#"
        SELECT
            owner_refno,
            owner_type,
            in as refno,
            in.noun as noun,
            spec_value as spec_value
        FROM inst_relate
        WHERE dbnum = $dbnum
    "#;

    let mut resp = aios_core::project_primary_db()
        .query(sql)
        .bind(("dbnum", dbnum))
        .await?;
    let rows: Vec<InstRelateRow> = resp.take(0)?;

    if verbose {
        println!("   ✅ inst_relate 命中: {} 条", rows.len());
    }

    Ok(rows)
}

/// 查询 inst_relate 全表（不按 dbnum 过滤）
async fn query_inst_relate_all(verbose: bool) -> Result<Vec<InstRelateRow>> {
    if verbose {
        println!("🔍 分批扫描 inst_relate 全表...");
    }

    const PAGE_SIZE: usize = 20_000;
    let mut offset = 0usize;
    let mut rows = Vec::new();

    loop {
        let sql = format!(
            r#"
        SELECT
            owner_refno,
            owner_type,
            in as refno,
            in.noun as noun,
            spec_value as spec_value
        FROM inst_relate
        ORDER BY in
        LIMIT {PAGE_SIZE} START {offset}
    "#
        );

        let batch: Vec<InstRelateRow> = aios_core::project_primary_db().query_take(&sql, 0).await?;
        if batch.is_empty() {
            break;
        }

        if verbose {
            println!(
                "   - inst_relate 分页: offset={} 本批={}",
                offset,
                batch.len()
            );
        }

        offset += PAGE_SIZE;
        rows.extend(batch);
    }

    if verbose {
        println!("   ✅ inst_relate 全表命中: {} 条", rows.len());
    }

    Ok(rows)
}

async fn query_export_insts(
    refnos: &[RefnoEnum],
    enable_holes: bool,
) -> Result<Vec<aios_core::ExportInstQuery>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let batch_size = 50;
    let mut results = Vec::new();

    for chunk in refnos.chunks(batch_size) {
        if enable_holes {
            let bool_keys = chunk
                .iter()
                .map(|r| format!("inst_relate_bool:{r}"))
                .collect::<Vec<_>>();
            let bool_keys_str = bool_keys.join(",");

            let bool_sql = format!(
                r#"
                SELECT
                    refno,
                    refno.owner as owner,
                    (if type::record("inst_relate_aabb", record::id(refno)).aabb_id != NONE {{
                        record::id(type::record("inst_relate_aabb", record::id(refno)).aabb_id)
                    }} else {{ None }}) as world_aabb_hash,
                    (if type::record("pe_transform", record::id(refno)).world_trans != NONE {{
                        record::id(type::record("pe_transform", record::id(refno)).world_trans)
                    }} else {{ None }}) as world_trans_hash,
                    [{{ "geo_hash": mesh_id, "trans_hash": "0", "unit_flag": false }}] as insts,
                    true as has_neg
                FROM [{bool_keys}]
                WHERE status = 'Success'
                  AND type::record("pe_transform", record::id(refno)).world_trans.d != NONE
                "#,
                bool_keys = bool_keys_str
            );

            let mut bool_results: Vec<aios_core::ExportInstQuery> = aios_core::project_primary_db()
                .query_take(&bool_sql, 0)
                .await
                .with_context(|| "query_export_insts bool SQL failed")?;

            let bool_refnos: HashSet<RefnoEnum> = bool_results.iter().map(|r| r.refno).collect();
            results.append(&mut bool_results);

            let non_bool_keys = chunk
                .iter()
                .filter(|r| !bool_refnos.contains(*r))
                .map(|r| r.to_inst_relate_key())
                .collect::<Vec<_>>();

            if !non_bool_keys.is_empty() {
                let non_bool_keys_str = non_bool_keys.join(",");
                let geo_sql = format!(
                    r#"
                    SELECT
                        in as refno,
                        in.owner ?? in as owner,
                        (if type::record("inst_relate_aabb", record::id(in)).aabb_id != NONE {{
                            record::id(type::record("inst_relate_aabb", record::id(in)).aabb_id)
                        }} else {{ None }}) as world_aabb_hash,
                        (if type::record("pe_transform", record::id(in)).world_trans != NONE {{
                            record::id(type::record("pe_transform", record::id(in)).world_trans)
                        }} else {{ None }}) as world_trans_hash,
                        (
                            SELECT
                                record::id(trans) as trans_hash,
                                record::id(out) as geo_hash,
                                out.unit_flag ?? false as unit_flag
                            FROM $parent.out->geo_relate
                            WHERE visible
                              && (trans.d ?? NONE) != NONE
                              && geo_type IN ['Pos', 'CatePos', 'Compound', 'Neg']
                        ) as insts,
                        false as has_neg
                    FROM [{non_bool_keys}]
                    WHERE type::record("pe_transform", record::id(in)).world_trans.d != NONE
                    "#,
                    non_bool_keys = non_bool_keys_str
                );

                let mut geo_results: Vec<aios_core::ExportInstQuery> =
                    aios_core::project_primary_db()
                        .query_take(&geo_sql, 0)
                        .await
                        .with_context(|| "query_export_insts geo SQL failed")?;
                results.append(&mut geo_results);
            }
        } else {
            let keys = chunk
                .iter()
                .map(|r| r.to_inst_relate_key())
                .collect::<Vec<_>>();
            let keys_str = keys.join(",");

            let sql = format!(
                r#"
                SELECT
                    in as refno,
                    in.owner ?? in as owner,
                    (if type::record("inst_relate_aabb", record::id(in)).aabb_id != NONE {{
                        record::id(type::record("inst_relate_aabb", record::id(in)).aabb_id)
                    }} else {{ None }}) as world_aabb_hash,
                    (if type::record("pe_transform", record::id(in)).world_trans != NONE {{
                        record::id(type::record("pe_transform", record::id(in)).world_trans)
                    }} else {{ None }}) as world_trans_hash,
                    (
                        SELECT
                            record::id(trans) as trans_hash,
                            record::id(out) as geo_hash,
                            out.unit_flag ?? false as unit_flag
                        FROM $parent.out->geo_relate
                        WHERE visible
                          && (trans.d ?? NONE) != NONE
                          && geo_type IN ['Pos', 'DesiPos', 'CatePos', 'Compound', 'Neg']
                    ) as insts,
                    false as has_neg
                FROM [{keys}]
                WHERE type::record("pe_transform", record::id(in)).world_trans.d != NONE
                "#,
                keys = keys_str
            );

            let mut chunk_results: Vec<aios_core::ExportInstQuery> =
                aios_core::project_primary_db()
                    .query_take(&sql, 0)
                    .await
                    .with_context(|| "query_export_insts SQL failed")?;
            results.append(&mut chunk_results);
        }
    }

    Ok(results)
}

async fn query_tubi_relate(
    owner_refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<HashMap<RefnoEnum, Vec<TubiQueryRow>>> {
    let mut tubings_map: HashMap<RefnoEnum, Vec<TubiQueryRow>> = HashMap::new();
    if owner_refnos.is_empty() {
        return Ok(tubings_map);
    }

    for owners_chunk in owner_refnos.chunks(200) {
        let mut sql_batch = String::new();
        for owner_refno in owners_chunk {
            let pe_key = owner_refno.to_pe_key();
            sql_batch.push_str(&format!(
                r#"
                SELECT
                    id[0] as refno,
                    id[1] as index,
                    in as leave,
                    record::id(aabb) as world_aabb_hash,
                    record::id(world_trans) as world_trans_hash,
                    record::id(geo) as geo_hash,
                    spec_value
                FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                "#,
            ));
        }

        let mut resp = aios_core::project_primary_db()
            .query_response(&sql_batch)
            .await?;
        for (stmt_idx, owner_refno) in owners_chunk.iter().enumerate() {
            let raw_rows: Vec<TubiQueryRow> = resp.take(stmt_idx)?;
            for row in raw_rows {
                if row.geo_hash.is_some() {
                    tubings_map.entry(*owner_refno).or_default().push(row);
                }
            }
        }
    }

    for tubis in tubings_map.values_mut() {
        tubis.sort_by_key(|t| t.index.unwrap_or(0));
    }

    if verbose {
        let total: usize = tubings_map.values().map(|v| v.len()).sum();
        println!("   ✅ tubi_relate: {} 条", total);
    }

    Ok(tubings_map)
}

// =============================================================================
// Trans / AABB 查询与转换
// =============================================================================

async fn resolve_trans_to_matrices(
    hashes: &HashSet<String>,
    unit_converter: &UnitConverter,
    apply_rotation: bool,
    verbose: bool,
) -> Result<HashMap<String, Vec<f32>>> {
    use aios_core::model_primary_db;

    let mut result = HashMap::new();
    if hashes.is_empty() {
        return Ok(result);
    }

    let rotation_mat = if apply_rotation {
        Some(DMat4::from_rotation_x(-std::f64::consts::FRAC_PI_2))
    } else {
        None
    };

    let factor = unit_converter.conversion_factor() as f64;
    let needs_conversion = unit_converter.needs_conversion();

    let hashes_vec: Vec<&String> = hashes.iter().collect();
    for chunk in hashes_vec.chunks(500) {
        let keys: Vec<String> = chunk.iter().map(|h| format!("trans:⟨{}⟩", h)).collect();
        let sql = format!(
            "SELECT record::id(id) as hash, d FROM [{}]",
            keys.join(", ")
        );

        let rows: Vec<TransQueryRow> = model_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();

        for row in rows {
            if let Some(mut mat) = parse_transform_to_dmat4(&row.d) {
                if needs_conversion {
                    let cols = mat.to_cols_array_2d();
                    mat = DMat4::from_cols_array_2d(&[
                        cols[0],
                        cols[1],
                        cols[2],
                        [
                            cols[3][0] * factor,
                            cols[3][1] * factor,
                            cols[3][2] * factor,
                            cols[3][3],
                        ],
                    ]);
                }

                if let Some(rot) = &rotation_mat {
                    mat = *rot * mat;
                }

                let cols: Vec<f32> = mat.to_cols_array().iter().map(|&v| v as f32).collect();
                result.insert(row.hash, cols);
            }
        }
    }

    if verbose {
        println!("   ✅ 解析到 {} 个 trans 矩阵", result.len());
    }

    Ok(result)
}

fn parse_transform_to_dmat4(d: &serde_json::Value) -> Option<DMat4> {
    let obj = d.as_object()?;

    let translation = obj
        .get("translation")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            glam::DVec3::new(x, y, z)
        })
        .unwrap_or(glam::DVec3::ZERO);

    let rotation = obj
        .get("rotation")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let w = arr.get(3).and_then(|v| v.as_f64()).unwrap_or(1.0);
            glam::DQuat::from_xyzw(x, y, z, w)
        })
        .unwrap_or(glam::DQuat::IDENTITY);

    let scale = obj
        .get("scale")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(1.0);
            let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0);
            let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0);
            glam::DVec3::new(x, y, z)
        })
        .unwrap_or(glam::DVec3::ONE);

    Some(DMat4::from_scale_rotation_translation(
        scale,
        rotation,
        translation,
    ))
}

async fn resolve_aabb(
    hashes: &HashSet<String>,
    unit_converter: &UnitConverter,
    verbose: bool,
) -> Result<HashMap<String, Vec<f64>>> {
    use aios_core::project_primary_db;

    let mut result = HashMap::new();
    if hashes.is_empty() {
        return Ok(result);
    }

    let factor = unit_converter.conversion_factor() as f64;

    let hashes_vec: Vec<&String> = hashes.iter().collect();
    for chunk in hashes_vec.chunks(500) {
        let keys: Vec<String> = chunk.iter().map(|h| format!("aabb:⟨{}⟩", h)).collect();
        let sql = format!(
            "SELECT record::id(id) as hash, d FROM [{}]",
            keys.join(", ")
        );

        let rows: Vec<AabbQueryRow> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();

        for row in rows {
            if let Some(aabb) = row.d {
                let mins = aabb.0.mins;
                let maxs = aabb.0.maxs;
                result.insert(
                    row.hash,
                    vec![
                        mins.x as f64 * factor,
                        mins.y as f64 * factor,
                        mins.z as f64 * factor,
                        maxs.x as f64 * factor,
                        maxs.y as f64 * factor,
                        maxs.z as f64 * factor,
                    ],
                );
            }
        }
    }

    if verbose {
        println!("   ✅ 解析到 {} 个 aabb", result.len());
    }

    Ok(result)
}

// =============================================================================
// 全库一次性导出（不按 dbnum 拆分）
// =============================================================================

/// 一次性查询 inst_relate 全表，直接输出单个 instances_v3.json。
/// 与 `export_dbnum_instances_v3` 逻辑完全一致，仅数据来源改为全表。
pub async fn export_all_instances_v3(
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    transform_config: ExportTransformConfig,
) -> Result<V3ExportStats> {
    let start_time = std::time::Instant::now();
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    if verbose {
        println!("🚀 [v3] 全库一次性导出（不按 dbnum 拆分）");
        if transform_config.needs_unit_conversion() {
            println!(
                "   单位转换: {} → {}",
                transform_config.source_unit.name(),
                transform_config.target_unit.name()
            );
        }
        if transform_config.apply_rotation {
            println!("   坐标旋转: Z-up → Y-up");
        }
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    // 1. 查询 inst_relate 全表
    {
        use crate::data_interface::db_meta;
        let _ = db_meta().ensure_loaded();
    }

    let inst_rows = query_inst_relate_all(verbose).await?;

    if verbose {
        println!("   ✅ inst_relate 记录: {} 条", inst_rows.len());
    }

    // 2. 按 owner 分组
    let mut bran_groups: HashMap<RefnoEnum, OwnerGroup> = HashMap::new();
    let mut equi_groups: HashMap<RefnoEnum, OwnerGroup> = HashMap::new();
    let mut ungrouped: Vec<UngroupedInfo> = Vec::new();
    let mut in_refnos: Vec<RefnoEnum> = Vec::new();
    let mut in_refno_set: HashSet<RefnoEnum> = HashSet::new();

    for row in &inst_rows {
        let owner_type = row
            .owner_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let spec_value = row.spec_value.unwrap_or(0);

        if in_refno_set.insert(row.refno) {
            in_refnos.push(row.refno);
        }

        match owner_type.as_str() {
            "BRAN" | "HANG" => {
                if let Some(owner) = row.owner_refno {
                    bran_groups
                        .entry(owner)
                        .or_insert_with(|| OwnerGroup {
                            owner_type: owner_type.clone(),
                            children: Vec::new(),
                        })
                        .children
                        .push(ChildInfo {
                            refno: row.refno,
                            noun: row.noun.clone().unwrap_or_default(),
                            spec_value,
                        });
                } else {
                    ungrouped.push(UngroupedInfo {
                        refno: row.refno,
                        noun: row.noun.clone().unwrap_or_default(),
                    });
                }
            }
            "EQUI" => {
                if let Some(owner) = row.owner_refno {
                    equi_groups
                        .entry(owner)
                        .or_insert_with(|| OwnerGroup {
                            owner_type: "EQUI".to_string(),
                            children: Vec::new(),
                        })
                        .children
                        .push(ChildInfo {
                            refno: row.refno,
                            noun: row.noun.clone().unwrap_or_default(),
                            spec_value,
                        });
                } else {
                    ungrouped.push(UngroupedInfo {
                        refno: row.refno,
                        noun: row.noun.clone().unwrap_or_default(),
                    });
                }
            }
            _ => {
                ungrouped.push(UngroupedInfo {
                    refno: row.refno,
                    noun: row.noun.clone().unwrap_or_default(),
                });
            }
        }
    }

    // 3. 查询几何体实例 hash
    if verbose {
        println!("🔍 查询 {} 个 refno 的几何体实例 hash...", in_refnos.len());
    }
    let mut export_inst_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();
    if !in_refnos.is_empty() {
        match query_export_insts(&in_refnos, true).await {
            Ok(export_insts) => {
                for inst in export_insts {
                    export_inst_map.insert(inst.refno, inst);
                }
                if verbose {
                    println!(
                        "   ✅ 有几何体的 refno: {}/{}",
                        export_inst_map.len(),
                        in_refnos.len()
                    );
                }
            }
            Err(e) => {
                if verbose {
                    println!("   ⚠️ 几何体实例查询失败: {:?}", e);
                }
            }
        }
    }

    // 4. 查询 tubi_relate
    let bran_owner_refnos: Vec<RefnoEnum> = bran_groups.keys().copied().collect();
    if verbose {
        println!(
            "🔍 查询 {} 个 BRAN/HANG owner 的 tubi_relate...",
            bran_owner_refnos.len()
        );
    }
    let tubings_map = query_tubi_relate(&bran_owner_refnos, verbose).await?;

    // 5. 收集所有唯一 trans_hash 和 aabb_hash
    let mut trans_hashes: HashSet<String> = HashSet::new();
    let mut aabb_hashes: HashSet<String> = HashSet::new();

    for export_inst in export_inst_map.values() {
        if let Some(ref h) = export_inst.world_trans_hash {
            if !h.is_empty() {
                trans_hashes.insert(h.clone());
            }
        }
        if let Some(ref h) = export_inst.world_aabb_hash {
            if !h.is_empty() {
                aabb_hashes.insert(h.clone());
            }
        }
        for inst in &export_inst.insts {
            if let Some(ref th) = inst.trans_hash {
                if !th.is_empty() {
                    trans_hashes.insert(th.clone());
                }
            }
        }
    }
    for tubis in tubings_map.values() {
        for tubi in tubis {
            if let Some(ref h) = tubi.world_trans_hash {
                if !h.is_empty() {
                    trans_hashes.insert(h.clone());
                }
            }
            if let Some(ref h) = tubi.world_aabb_hash {
                if !h.is_empty() {
                    aabb_hashes.insert(h.clone());
                }
            }
        }
    }

    // 6. 批量查询 trans 和 aabb 实际数据
    if verbose {
        println!(
            "🔍 查询 {} 个 trans, {} 个 aabb...",
            trans_hashes.len(),
            aabb_hashes.len()
        );
    }

    let unit_converter =
        UnitConverter::new(transform_config.source_unit, transform_config.target_unit);

    let (trans_map, aabb_map) = tokio::join!(
        resolve_trans_to_matrices(
            &trans_hashes,
            &unit_converter,
            transform_config.apply_rotation,
            verbose
        ),
        resolve_aabb(&aabb_hashes, &unit_converter, verbose),
    );
    let trans_map = trans_map?;
    let aabb_map = aabb_map?;

    if verbose {
        println!(
            "   ✅ trans 命中: {}, aabb 命中: {}",
            trans_map.len(),
            aabb_map.len()
        );
    }

    // 7. 构建 JSON
    let mut total_component_instances: usize = 0;
    let mut total_tubing_instances: usize = 0;

    let transforms_json: serde_json::Map<String, serde_json::Value> = trans_map
        .iter()
        .map(|(hash, cols)| (hash.clone(), json!(cols)))
        .collect();

    let aabb_json: serde_json::Map<String, serde_json::Value> = aabb_map
        .iter()
        .map(|(hash, vals)| (hash.clone(), json!(vals)))
        .collect();

    // --- bran_groups ---
    let mut bran_groups_json: Vec<serde_json::Value> = Vec::new();
    let mut bran_keys: Vec<RefnoEnum> = bran_groups.keys().copied().collect();
    bran_keys.sort();

    for owner_refno in &bran_keys {
        let group = &bran_groups[owner_refno];
        let mut children_json: Vec<serde_json::Value> = Vec::new();

        for child in &group.children {
            let export_inst = match export_inst_map.get(&child.refno) {
                Some(ei) if !ei.insts.is_empty() => ei,
                _ => continue,
            };

            let mut geos_json: Vec<serde_json::Value> = Vec::new();
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                geos_json.push(json!({
                    "geo_hash": inst.geo_hash,
                    "geo_index": geo_idx,
                    "geo_trans_hash": inst.trans_hash.as_deref().unwrap_or("0"),
                    "unit_mesh": inst.unit_flag,
                }));
                total_component_instances += 1;
            }

            children_json.push(json!({
                "refno": child.refno.to_string(),
                "noun": child.noun,
                "owner_noun": group.owner_type,
                "trans_hash": export_inst.world_trans_hash.as_deref().unwrap_or(""),
                "aabb_hash": export_inst.world_aabb_hash.as_deref().unwrap_or(""),
                "spec_value": child.spec_value,
                "has_neg": export_inst.has_neg,
                "geos": geos_json,
            }));
        }

        // TUBI
        let mut tubings_json: Vec<serde_json::Value> = Vec::new();
        if let Some(tubis) = tubings_map.get(owner_refno) {
            for tubi in tubis {
                tubings_json.push(json!({
                    "refno": tubi.leave.to_string(),
                    "owner_refno": owner_refno.to_string(),
                    "order": tubi.index.unwrap_or(0),
                    "geo_hash": tubi.geo_hash.as_deref().unwrap_or(""),
                    "trans_hash": tubi.world_trans_hash.as_deref().unwrap_or(""),
                    "aabb_hash": tubi.world_aabb_hash.as_deref().unwrap_or(""),
                    "spec_value": tubi.spec_value.unwrap_or(0),
                }));
                total_tubing_instances += 1;
            }
        }

        bran_groups_json.push(json!({
            "refno": owner_refno.to_string(),
            "noun": group.owner_type,
            "children": children_json,
            "tubings": tubings_json,
        }));
    }

    // --- equi_groups ---
    let mut equi_groups_json: Vec<serde_json::Value> = Vec::new();
    let mut equi_keys: Vec<RefnoEnum> = equi_groups.keys().copied().collect();
    equi_keys.sort();

    for owner_refno in &equi_keys {
        let group = &equi_groups[owner_refno];
        let mut children_json: Vec<serde_json::Value> = Vec::new();

        for child in &group.children {
            let export_inst = match export_inst_map.get(&child.refno) {
                Some(ei) if !ei.insts.is_empty() => ei,
                _ => continue,
            };

            let mut geos_json: Vec<serde_json::Value> = Vec::new();
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                geos_json.push(json!({
                    "geo_hash": inst.geo_hash,
                    "geo_index": geo_idx,
                    "geo_trans_hash": inst.trans_hash.as_deref().unwrap_or("0"),
                    "unit_mesh": inst.unit_flag,
                }));
                total_component_instances += 1;
            }

            children_json.push(json!({
                "refno": child.refno.to_string(),
                "noun": child.noun,
                "owner_noun": "EQUI",
                "trans_hash": export_inst.world_trans_hash.as_deref().unwrap_or(""),
                "aabb_hash": export_inst.world_aabb_hash.as_deref().unwrap_or(""),
                "spec_value": child.spec_value,
                "has_neg": export_inst.has_neg,
                "geos": geos_json,
            }));
        }

        equi_groups_json.push(json!({
            "refno": owner_refno.to_string(),
            "noun": "EQUI",
            "children": children_json,
        }));
    }

    // --- ungrouped ---
    let mut ungrouped_json: Vec<serde_json::Value> = Vec::new();
    for item in &ungrouped {
        let export_inst = match export_inst_map.get(&item.refno) {
            Some(ei) if !ei.insts.is_empty() => ei,
            _ => continue,
        };

        let mut geos_json: Vec<serde_json::Value> = Vec::new();
        for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
            geos_json.push(json!({
                "geo_hash": inst.geo_hash,
                "geo_index": geo_idx,
                "geo_trans_hash": inst.trans_hash.as_deref().unwrap_or("0"),
                "unit_mesh": inst.unit_flag,
            }));
            total_component_instances += 1;
        }

        ungrouped_json.push(json!({
            "refno": item.refno.to_string(),
            "noun": item.noun,
            "owner_noun": "",
            "trans_hash": export_inst.world_trans_hash.as_deref().unwrap_or(""),
            "aabb_hash": export_inst.world_aabb_hash.as_deref().unwrap_or(""),
            "spec_value": 0,
            "has_neg": export_inst.has_neg,
            "geos": geos_json,
        }));
    }

    // 8. 组装并写入文件
    let output_filename = "instances_v3.json".to_string();

    let root_json = json!({
        "version": 3,
        "format": "json",
        "generated_at": generated_at,
        "scope": "all",
        "export_transform": transform_config.to_manifest_json(),
        "transforms": transforms_json,
        "aabb": aabb_json,
        "bran_groups": bran_groups_json,
        "equi_groups": equi_groups_json,
        "ungrouped": ungrouped_json,
    });

    let output_path = output_dir.join(&output_filename);
    let json_str = serde_json::to_string(&root_json)?;
    std::fs::write(&output_path, &json_str)?;

    let elapsed = start_time.elapsed();

    if verbose {
        let file_size = std::fs::metadata(&output_path)
            .map(|m| m.len())
            .unwrap_or(0);
        println!("\n📊 [v3-all] 全库导出统计:");
        println!("   - BRAN/HANG 分组: {}", bran_groups_json.len());
        println!("   - EQUI 分组: {}", equi_groups_json.len());
        println!("   - 未分组: {}", ungrouped_json.len());
        println!("   - 构件实例: {}", total_component_instances);
        println!("   - TUBI 实例: {}", total_tubing_instances);
        println!("   - transforms 条目: {}", transforms_json.len());
        println!("   - aabb 条目: {}", aabb_json.len());
        println!("   - 文件大小: {:.2} MB", file_size as f64 / 1_048_576.0);
        println!("   - 耗时: {:.2}s", elapsed.as_secs_f64());
        println!("   ✅ 写入: {}", output_path.display());
    }

    Ok(V3ExportStats {
        bran_group_count: bran_groups_json.len(),
        equi_group_count: equi_groups_json.len(),
        ungrouped_count: ungrouped_json.len(),
        total_component_instances,
        total_tubing_instances,
        transform_count: transforms_json.len(),
        aabb_count: aabb_json.len(),
        elapsed,
        output_filename,
    })
}

// =============================================================================
// 合并所有 per-dbnum V3 JSON 为单文件
// =============================================================================

pub struct V3MergeStats {
    pub file_count: usize,
    pub bran_group_count: usize,
    pub equi_group_count: usize,
    pub ungrouped_count: usize,
    pub transform_count: usize,
    pub aabb_count: usize,
    pub output_size_bytes: u64,
    pub elapsed: std::time::Duration,
}

/// 读取 `v3_bundle_dir` 下所有 `instances_v3_<dbnum>.json`，
/// 合并 transforms/aabb 字典 + 拼接 bran_groups/equi_groups/ungrouped，
/// 写入 `instances_v3.json`。纯文件操作，无需数据库连接。
pub fn merge_v3_instances(v3_bundle_dir: &Path, verbose: bool) -> Result<V3MergeStats> {
    let start = std::time::Instant::now();

    // 1. 扫描 per-dbnum 文件
    let mut per_dbnum_files: Vec<std::path::PathBuf> = Vec::new();
    for entry in std::fs::read_dir(v3_bundle_dir)
        .with_context(|| format!("读取目录失败: {}", v3_bundle_dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // 匹配 instances_v3_<digits>.json，排除 root_ 文件和已合并的 instances_v3.json
        if name_str.starts_with("instances_v3_")
            && name_str.ends_with(".json")
            && !name_str.contains("root_")
            && name_str != "instances_v3.json"
        {
            per_dbnum_files.push(entry.path());
        }
    }
    per_dbnum_files.sort();

    if per_dbnum_files.is_empty() {
        anyhow::bail!(
            "在 {} 下未找到 instances_v3_<dbnum>.json 文件",
            v3_bundle_dir.display()
        );
    }

    if verbose {
        println!(
            "📂 找到 {} 个 per-dbnum V3 JSON 文件",
            per_dbnum_files.len()
        );
    }

    // 2. 逐文件读取并合并
    let mut merged_transforms: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut merged_aabb: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut merged_bran_groups: Vec<serde_json::Value> = Vec::new();
    let mut merged_equi_groups: Vec<serde_json::Value> = Vec::new();
    let mut merged_ungrouped: Vec<serde_json::Value> = Vec::new();
    let mut first_export_transform: Option<serde_json::Value> = None;
    let mut dbnums: Vec<u32> = Vec::new();
    let mut skipped = 0usize;

    for path in &per_dbnum_files {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取失败: {}", path.display()))?;
        let root: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("JSON 解析失败: {}", path.display()))?;

        let obj = match root.as_object() {
            Some(o) => o,
            None => {
                if verbose {
                    println!("   ⚠️ 跳过非对象 JSON: {}", path.display());
                }
                skipped += 1;
                continue;
            }
        };

        // dbnum
        if let Some(dbnum) = obj.get("dbnum").and_then(|v| v.as_u64()) {
            dbnums.push(dbnum as u32);
        }

        // export_transform（取第一个非空的）
        if first_export_transform.is_none() {
            if let Some(et) = obj.get("export_transform") {
                first_export_transform = Some(et.clone());
            }
        }

        // transforms
        if let Some(transforms) = obj.get("transforms").and_then(|v| v.as_object()) {
            for (k, v) in transforms {
                merged_transforms
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
        }

        // aabb
        if let Some(aabb) = obj.get("aabb").and_then(|v| v.as_object()) {
            for (k, v) in aabb {
                merged_aabb.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }

        // bran_groups
        if let Some(groups) = obj.get("bran_groups").and_then(|v| v.as_array()) {
            merged_bran_groups.extend(groups.iter().cloned());
        }

        // equi_groups
        if let Some(groups) = obj.get("equi_groups").and_then(|v| v.as_array()) {
            merged_equi_groups.extend(groups.iter().cloned());
        }

        // ungrouped
        if let Some(items) = obj.get("ungrouped").and_then(|v| v.as_array()) {
            merged_ungrouped.extend(items.iter().cloned());
        }

        if verbose {
            let fname = path.file_name().unwrap_or_default().to_string_lossy();
            let t_count = obj
                .get("transforms")
                .and_then(|v| v.as_object())
                .map(|m| m.len())
                .unwrap_or(0);
            let b_count = obj
                .get("bran_groups")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let e_count = obj
                .get("equi_groups")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            println!(
                "   ✅ {} → trans={}, bran={}, equi={}",
                fname, t_count, b_count, e_count
            );
        }
    }

    if verbose && skipped > 0 {
        println!("   ⚠️ 跳过 {} 个无效文件", skipped);
    }

    // 3. 组装合并后的 JSON
    let generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let merged_json = json!({
        "version": 3,
        "format": "json",
        "generated_at": generated_at,
        "dbnums": dbnums,
        "export_transform": first_export_transform.unwrap_or(json!({})),
        "transforms": merged_transforms,
        "aabb": merged_aabb,
        "bran_groups": merged_bran_groups,
        "equi_groups": merged_equi_groups,
        "ungrouped": merged_ungrouped,
    });

    // 4. 写入
    let output_path = v3_bundle_dir.join("instances_v3.json");
    let json_str = serde_json::to_string_pretty(&merged_json)?;
    std::fs::write(&output_path, &json_str)
        .with_context(|| format!("写入合并文件失败: {}", output_path.display()))?;

    let output_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let elapsed = start.elapsed();

    if verbose {
        println!("\n📊 [v3-merge] 合并统计:");
        println!("   - 输入文件: {}", per_dbnum_files.len());
        println!("   - dbnums: {:?}", dbnums);
        println!("   - transforms 条目: {}", merged_transforms.len());
        println!("   - aabb 条目: {}", merged_aabb.len());
        println!("   - BRAN 分组: {}", merged_bran_groups.len());
        println!("   - EQUI 分组: {}", merged_equi_groups.len());
        println!("   - 未分组: {}", merged_ungrouped.len());
        println!("   - 文件大小: {:.2} MB", output_size as f64 / 1_048_576.0);
        println!("   - 耗时: {:.2}s", elapsed.as_secs_f64());
        println!("   ✅ 写入: {}", output_path.display());
    }

    Ok(V3MergeStats {
        file_count: per_dbnum_files.len(),
        bran_group_count: merged_bran_groups.len(),
        equi_group_count: merged_equi_groups.len(),
        ungrouped_count: merged_ungrouped.len(),
        transform_count: merged_transforms.len(),
        aabb_count: merged_aabb.len(),
        output_size_bytes: output_size,
        elapsed,
    })
}
