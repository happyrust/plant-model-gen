//! delivery-code 兼容的 V2 格式 instances.json 导出
//!
//! 从 SurrealDB 查询 inst_relate / geo_relate / tubi_relate / trans 数据，
//! 解析 trans hash 为实际 4×4 矩阵，组合 world × geo 变换，
//! 生成可被 delivery-code 的 DTXPrepackLoader / AiosPrepackLoader 直接消费的 JSON。
//!
//! 输出特点：
//! - 内联 4×4 矩阵（列主序 f32 数组），不使用 hash 引用
//! - 每个 instance 携带 `uniforms` 块（refno, owner_refno, owner_noun）
//! - 不做单位转换或坐标系旋转（前端自行处理）
//! - 顶层 `names` 表用于 name_index 查找

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aios_core::options::DbOption;
use aios_core::pdms_types::RefnoEnum;
use aios_core::SurrealQueryExt;
use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use glam::DMat4;
use serde::{Deserialize, Serialize};
use serde_json::json;
use surrealdb::types::SurrealValue;

use crate::fast_model::gen_model::tree_index_manager::{
    TreeIndexManager, load_index_with_large_stack,
};

use super::InstRelateRow;

// =============================================================================
// 公共返回类型
// =============================================================================

pub struct WebExportStats {
    pub bran_group_count: usize,
    pub equi_group_count: usize,
    pub ungrouped_count: usize,
    pub total_component_instances: usize,
    pub total_tubing_instances: usize,
    pub elapsed: std::time::Duration,
    /// 写入的主文件名（仅文件名，不含目录）
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
    pub world_trans_hash: Option<String>,
    pub geo_hash: Option<String>,
    pub spec_value: Option<i64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct TransQueryRow {
    hash: String,
    d: serde_json::Value,
}

// =============================================================================
// 内部数据结构
// =============================================================================

struct OwnerGroup {
    owner_type: String,
    owner_name: Option<String>,
    children: Vec<ChildInfo>,
}

struct ChildInfo {
    refno: RefnoEnum,
    noun: String,
    name: Option<String>,
    spec_value: i64,
}

struct TubiInfo {
    leave_refno: RefnoEnum,
    owner_refno: RefnoEnum,
    order: usize,
    geo_hash: String,
    trans_hash: Option<String>,
    spec_value: i64,
}

struct UngroupedInfo {
    refno: RefnoEnum,
    noun: String,
    name: Option<String>,
}

// names 表条目
struct NameEntry {
    kind: &'static str,
    value: String,
}

struct NameTable {
    entries: Vec<NameEntry>,
    index_map: HashMap<String, usize>,
}

impl NameTable {
    fn new() -> Self {
        let mut table = Self {
            entries: Vec::new(),
            index_map: HashMap::new(),
        };
        table.insert("site", "UNKNOWN_SITE");
        table
    }

    fn insert(&mut self, kind: &'static str, value: &str) -> usize {
        let key = format!("{}:{}", kind, value);
        if let Some(&idx) = self.index_map.get(&key) {
            return idx;
        }
        let idx = self.entries.len();
        self.entries.push(NameEntry {
            kind,
            value: value.to_string(),
        });
        self.index_map.insert(key, idx);
        idx
    }

    fn to_json(&self) -> Vec<serde_json::Value> {
        self.entries
            .iter()
            .map(|e| {
                json!({
                    "kind": e.kind,
                    "value": e.value,
                })
            })
            .collect()
    }
}

// =============================================================================
// Trans 解析
// =============================================================================

/// 批量解析 trans hash → DMat4（不做单位转换）
async fn resolve_trans_to_matrices(
    hashes: &HashSet<String>,
    verbose: bool,
) -> Result<HashMap<String, DMat4>> {
    use aios_core::model_primary_db;

    let mut result = HashMap::new();
    if hashes.is_empty() {
        return Ok(result);
    }

    let hashes_vec: Vec<&String> = hashes.iter().collect();
    for chunk in hashes_vec.chunks(500) {
        let keys: Vec<String> = chunk.iter().map(|h| format!("trans:⟨{}⟩", h)).collect();
        let sql = format!(
            "SELECT record::id(id) as hash, d FROM [{}]",
            keys.join(", ")
        );

        if verbose {
            println!("   查询 trans: {} 个", chunk.len());
        }

        let rows: Vec<TransQueryRow> = model_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();

        for row in rows {
            if let Some(mat) = parse_transform_to_dmat4(&row.d) {
                result.insert(row.hash, mat);
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

/// DMat4 → 列主序 f32 数组
fn dmat4_to_f32_array(mat: &DMat4) -> Vec<f32> {
    mat.to_cols_array().iter().map(|&v| v as f32).collect()
}

/// 组合世界矩阵和几何体局部矩阵
///
/// - 非 has_neg: result = world_trans × geo_trans
/// - has_neg: geo_trans 已包含世界变换信息，直接使用 geo_trans
fn combine_world_geo_matrix(
    world_mat: &DMat4,
    geo_mat: &DMat4,
    has_neg: bool,
) -> Vec<f32> {
    let combined = if has_neg {
        *geo_mat
    } else {
        *world_mat * *geo_mat
    };
    dmat4_to_f32_array(&combined)
}

// =============================================================================
// LOD Mask 计算
// =============================================================================

fn compute_lod_mask_from_disk(geo_hash: &str, mesh_base_dir: &Path) -> u32 {
    let mut mask = 0u32;
    for (level, tag) in [(1u32, "L1"), (2, "L2"), (3, "L3")] {
        let lod_dir = mesh_base_dir.join(format!("lod_{}", tag));
        let candidates = [
            lod_dir.join(format!("{}_{}.glb", geo_hash, tag)),
            lod_dir.join(format!("{}.glb", geo_hash)),
        ];
        if candidates.iter().any(|p| p.exists()) {
            mask |= 1 << (level - 1);
        }
    }
    if mask == 0 {
        7 // 默认所有 LOD 可用
    } else {
        mask
    }
}

// =============================================================================
// SurrealDB 查询
// =============================================================================

async fn query_tubi_relate_web(
    owner_refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<HashMap<RefnoEnum, Vec<TubiInfo>>> {
    let mut tubings_map: HashMap<RefnoEnum, Vec<TubiInfo>> = HashMap::new();
    if owner_refnos.is_empty() {
        return Ok(tubings_map);
    }

    for owners_chunk in owner_refnos.chunks(50) {
        let mut sql_batch = String::new();
        for owner_refno in owners_chunk {
            let pe_key = owner_refno.to_pe_key();
            sql_batch.push_str(&format!(
                r#"
                SELECT
                    id[0] as refno,
                    id[1] as index,
                    in as leave,
                    record::id(world_trans) as world_trans_hash,
                    record::id(geo) as geo_hash,
                    spec_value
                FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                "#,
            ));
        }

        let mut resp = aios_core::model_primary_db()
            .query_response(&sql_batch)
            .await?;

        for (stmt_idx, owner_refno) in owners_chunk.iter().enumerate() {
            let raw_rows: Vec<TubiQueryRow> = resp.take(stmt_idx)?;
            for row in raw_rows {
                let Some(geo_hash) = row.geo_hash else {
                    continue;
                };
                let order = row
                    .index
                    .and_then(|v| usize::try_from(v).ok())
                    .unwrap_or(0);

                tubings_map
                    .entry(*owner_refno)
                    .or_default()
                    .push(TubiInfo {
                        leave_refno: row.leave,
                        owner_refno: *owner_refno,
                        order,
                        geo_hash,
                        trans_hash: row.world_trans_hash,
                        spec_value: row.spec_value.unwrap_or(0),
                    });
            }
        }
    }

    for tubis in tubings_map.values_mut() {
        tubis.sort_by_key(|t| t.order);
    }

    if verbose {
        let total: usize = tubings_map.values().map(|v| v.len()).sum();
        println!("   ✅ 查询到 {} 条 tubi_relate 记录", total);
    }

    Ok(tubings_map)
}

// =============================================================================
// 主导出函数
// =============================================================================

/// 导出 delivery-code 兼容的 V2 格式 instances.json
///
/// 不做单位转换或坐标系旋转，矩阵保持 SurrealDB 原始数据。
/// 前端根据 manifest.json 的 unit_conversion 字段自行处理 mm→m 转换和 Z-up→Y-up 旋转。
pub async fn export_dbnum_instances_web(
    dbnum: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    root_refno: Option<RefnoEnum>,
    mesh_base_dir: Option<PathBuf>,
) -> Result<WebExportStats> {
    let start_time = std::time::Instant::now();

    if verbose {
        println!(
            "🚀 [web] 开始导出 dbnum={} 的 V2 格式 instances.json",
            dbnum
        );
    }

    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    let mesh_dir = mesh_base_dir.unwrap_or_else(|| {
        db_option
            .meshes_path
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("assets/meshes"))
    });

    // =========================================================================
    // 1. 加载 TreeIndex → 获取 refno 列表
    // =========================================================================
    if verbose {
        println!("🔍 加载 TreeIndex...");
    }

    let tree_manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let tree_dir = tree_manager.tree_dir().to_path_buf();

    let all_refnos: Vec<RefnoEnum> = if let Some(root) = root_refno {
        use crate::fast_model::query_compat::query_deep_visible_inst_refnos;
        if verbose {
            println!("🔍 仅导出子树: {}（可见实例 refno）", root);
        }
        let mut list = query_deep_visible_inst_refnos(root).await?;
        let set: HashSet<RefnoEnum> = list.iter().copied().collect();
        if !set.contains(&root) {
            list.push(root);
        }
        list.sort_by_key(|r| r.to_string());
        list
    } else {
        let tree_index = load_index_with_large_stack(&tree_dir, dbnum)
            .with_context(|| format!("加载 TreeIndex 失败: dbnum={}", dbnum))?;
        tree_index
            .all_refnos()
            .into_iter()
            .map(RefnoEnum::from)
            .collect()
    };

    if verbose {
        println!("✅ refno 数量: {}", all_refnos.len());
    }

    // =========================================================================
    // 2. 查询 inst_relate → 分组
    // =========================================================================
    if verbose {
        println!("🔍 查询 inst_relate...");
    }

    let inst_rows = super::query_inst_relate_batch(&all_refnos, true, verbose).await?;

    if verbose {
        println!("✅ inst_relate 命中记录: {}", inst_rows.len());
    }

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

        if matches!(owner_type.as_str(), "BRAN" | "HANG") {
            if let Some(owner_refno) = row.owner_refno {
                if in_refno_set.insert(row.refno) {
                    in_refnos.push(row.refno);
                }
                let entry = bran_groups
                    .entry(owner_refno)
                    .or_insert_with(|| OwnerGroup {
                        owner_type: owner_type.clone(),
                        owner_name: None,
                        children: Vec::new(),
                    });
                entry.children.push(ChildInfo {
                    refno: row.refno,
                    noun: row.noun.clone().unwrap_or_default(),
                    name: row.name.clone(),
                    spec_value,
                });
            }
        } else if matches!(owner_type.as_str(), "EQUI") {
            if let Some(owner_refno) = row.owner_refno {
                if in_refno_set.insert(row.refno) {
                    in_refnos.push(row.refno);
                }
                let entry = equi_groups
                    .entry(owner_refno)
                    .or_insert_with(|| OwnerGroup {
                        owner_type: "EQUI".to_string(),
                        owner_name: None,
                        children: Vec::new(),
                    });
                entry.children.push(ChildInfo {
                    refno: row.refno,
                    noun: row.noun.clone().unwrap_or_default(),
                    name: row.name.clone(),
                    spec_value,
                });
            }
        } else {
            if in_refno_set.insert(row.refno) {
                in_refnos.push(row.refno);
            }
            ungrouped.push(UngroupedInfo {
                refno: row.refno,
                noun: row.noun.clone().unwrap_or_default(),
                name: row.name.clone(),
            });
        }
    }

    // =========================================================================
    // 3. 查询几何体实例 hash
    // =========================================================================
    if verbose {
        println!(
            "🔍 查询 {} 个 refno 的几何体实例 hash...",
            in_refnos.len()
        );
    }

    let mut export_inst_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();
    if !in_refnos.is_empty() {
        match aios_core::query_insts_for_export(&in_refnos, true).await {
            Ok(export_insts) => {
                for inst in export_insts {
                    export_inst_map.insert(inst.refno, inst);
                }
                if verbose {
                    println!("✅ 查询到 {} 个 refno 有几何体实例", export_inst_map.len());
                }
            }
            Err(e) => {
                eprintln!("⚠️ 几何体实例查询失败: {:?}", e);
            }
        }
    }

    // =========================================================================
    // 4. 查询 tubi_relate
    // =========================================================================
    let tubi_owner_refnos: Vec<RefnoEnum> = bran_groups.keys().copied().collect();

    if verbose {
        println!(
            "🔍 查询 {} 个 BRAN/HANG owner 的 tubi_relate...",
            tubi_owner_refnos.len()
        );
    }

    let tubings_map = query_tubi_relate_web(&tubi_owner_refnos, verbose).await?;

    // =========================================================================
    // 5. 收集所有 trans hash 并批量解析为矩阵
    // =========================================================================
    let mut trans_hashes: HashSet<String> = HashSet::new();

    for export_inst in export_inst_map.values() {
        if let Some(ref h) = export_inst.world_trans_hash {
            if !h.is_empty() {
                trans_hashes.insert(h.clone());
            }
        }
        for inst in &export_inst.insts {
            if let Some(ref h) = inst.trans_hash {
                if !h.is_empty() {
                    trans_hashes.insert(h.clone());
                }
            }
        }
    }

    for tubis in tubings_map.values() {
        for tubi in tubis {
            if let Some(ref h) = tubi.trans_hash {
                if !h.is_empty() {
                    trans_hashes.insert(h.clone());
                }
            }
        }
    }

    if verbose {
        println!("🔍 批量解析 {} 个 trans hash...", trans_hashes.len());
    }

    let trans_matrices = resolve_trans_to_matrices(&trans_hashes, verbose).await?;

    // =========================================================================
    // 6. 构建 V2 JSON
    // =========================================================================
    if verbose {
        println!("🔨 构建 V2 JSON...");
    }

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut name_table = NameTable::new();
    let mut total_component_instances = 0usize;
    let mut total_tubing_instances = 0usize;

    // 辅助：解析矩阵
    let resolve_mat = |hash: &Option<String>| -> DMat4 {
        hash.as_ref()
            .and_then(|h| trans_matrices.get(h))
            .copied()
            .unwrap_or(DMat4::IDENTITY)
    };

    // --- 构建 bran_groups ---
    let mut bran_groups_json: Vec<serde_json::Value> = Vec::new();
    let mut bran_keys: Vec<RefnoEnum> = bran_groups.keys().copied().collect();
    bran_keys.sort();

    for owner_refno in &bran_keys {
        let group = &bran_groups[owner_refno];
        let owner_name_str = format!("{}-{}", group.owner_type, owner_refno);
        let owner_name_index = name_table.insert("bran", &owner_name_str);

        let mut children_json: Vec<serde_json::Value> = Vec::new();

        for child in &group.children {
            let export_inst = match export_inst_map.get(&child.refno) {
                Some(ei) if !ei.insts.is_empty() => ei,
                _ => continue,
            };

            let fallback_name = child.refno.to_string();
            let child_name_value = child.name.as_deref().unwrap_or(&fallback_name);
            let child_name_index = name_table.insert("component", child_name_value);

            let world_mat = resolve_mat(&export_inst.world_trans_hash);
            let has_neg = export_inst.has_neg;

            let mut instances_json: Vec<serde_json::Value> = Vec::new();
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                let geo_mat = resolve_mat(&inst.trans_hash);
                let matrix = combine_world_geo_matrix(&world_mat, &geo_mat, has_neg);
                let lod_mask = compute_lod_mask_from_disk(&inst.geo_hash, &mesh_dir);

                instances_json.push(json!({
                    "geo_hash": inst.geo_hash,
                    "geo_index": geo_idx,
                    "matrix": matrix,
                    "name_index": child_name_index,
                    "site_name_index": 0,
                    "lod_mask": lod_mask,
                    "uniforms": {
                        "refno": child.refno.to_string(),
                        "owner_refno": owner_refno.to_string(),
                        "owner_noun": group.owner_type,
                    }
                }));
                total_component_instances += 1;
            }

            children_json.push(json!({
                "refno": child.refno.to_string(),
                "noun": child.noun,
                "name": child.name,
                "name_index": child_name_index,
                "instances": instances_json,
            }));
        }

        // --- tubings ---
        let mut tubings_json: Vec<serde_json::Value> = Vec::new();
        if let Some(tubis) = tubings_map.get(owner_refno) {
            for tubi in tubis {
                let tubi_name = format!("TUBI-{}-{}", tubi.leave_refno, tubi.order);
                let tubi_name_index = name_table.insert("tubi", &tubi_name);

                let world_mat = resolve_mat(&tubi.trans_hash);
                let matrix = dmat4_to_f32_array(&world_mat);
                let lod_mask = compute_lod_mask_from_disk(&tubi.geo_hash, &mesh_dir);

                tubings_json.push(json!({
                    "refno": tubi.leave_refno.to_string(),
                    "noun": "TUBI",
                    "name": tubi_name,
                    "geo_hash": tubi.geo_hash,
                    "geo_index": 0,
                    "matrix": matrix,
                    "name_index": tubi_name_index,
                    "site_name_index": 0,
                    "order": tubi.order,
                    "lod_mask": lod_mask,
                    "spec_value": tubi.spec_value,
                    "uniforms": {
                        "refno": tubi.leave_refno.to_string(),
                        "owner_refno": owner_refno.to_string(),
                        "owner_noun": group.owner_type,
                    }
                }));
                total_tubing_instances += 1;
            }
        }

        bran_groups_json.push(json!({
            "refno": owner_refno.to_string(),
            "noun": group.owner_type,
            "name": owner_name_str,
            "name_index": owner_name_index,
            "children": children_json,
            "tubings": tubings_json,
        }));
    }

    // --- 构建 equi_groups ---
    let mut equi_groups_json: Vec<serde_json::Value> = Vec::new();
    let mut equi_keys: Vec<RefnoEnum> = equi_groups.keys().copied().collect();
    equi_keys.sort();

    for owner_refno in &equi_keys {
        let group = &equi_groups[owner_refno];
        let owner_name_str = format!("EQUI-{}", owner_refno);
        let owner_name_index = name_table.insert("component", &owner_name_str);

        let mut children_json: Vec<serde_json::Value> = Vec::new();

        for child in &group.children {
            let export_inst = match export_inst_map.get(&child.refno) {
                Some(ei) if !ei.insts.is_empty() => ei,
                _ => continue,
            };

            let fallback_name = child.refno.to_string();
            let child_name_value = child.name.as_deref().unwrap_or(&fallback_name);
            let child_name_index = name_table.insert("component", child_name_value);

            let world_mat = resolve_mat(&export_inst.world_trans_hash);
            let has_neg = export_inst.has_neg;

            let mut instances_json: Vec<serde_json::Value> = Vec::new();
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                let geo_mat = resolve_mat(&inst.trans_hash);
                let matrix = combine_world_geo_matrix(&world_mat, &geo_mat, has_neg);
                let lod_mask = compute_lod_mask_from_disk(&inst.geo_hash, &mesh_dir);

                instances_json.push(json!({
                    "geo_hash": inst.geo_hash,
                    "geo_index": geo_idx,
                    "matrix": matrix,
                    "name_index": child_name_index,
                    "site_name_index": 0,
                    "lod_mask": lod_mask,
                    "uniforms": {
                        "refno": child.refno.to_string(),
                        "owner_refno": owner_refno.to_string(),
                        "owner_noun": "EQUI",
                    }
                }));
                total_component_instances += 1;
            }

            children_json.push(json!({
                "refno": child.refno.to_string(),
                "noun": child.noun,
                "name": child.name,
                "name_index": child_name_index,
                "instances": instances_json,
            }));
        }

        equi_groups_json.push(json!({
            "refno": owner_refno.to_string(),
            "noun": "EQUI",
            "name": owner_name_str,
            "name_index": owner_name_index,
            "children": children_json,
        }));
    }

    // --- 构建 ungrouped ---
    let mut ungrouped_json: Vec<serde_json::Value> = Vec::new();

    for item in &ungrouped {
        let export_inst = match export_inst_map.get(&item.refno) {
            Some(ei) if !ei.insts.is_empty() => ei,
            _ => continue,
        };

        let fallback_name = item.refno.to_string();
        let item_name_value = item.name.as_deref().unwrap_or(&fallback_name);
        let item_name_index = name_table.insert("component", item_name_value);

        let world_mat = resolve_mat(&export_inst.world_trans_hash);
        let has_neg = export_inst.has_neg;

        let owner_refno_str = export_inst
            .owner
            .to_string();

        let mut instances_json: Vec<serde_json::Value> = Vec::new();
        for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
            let geo_mat = resolve_mat(&inst.trans_hash);
            let matrix = combine_world_geo_matrix(&world_mat, &geo_mat, has_neg);
            let lod_mask = compute_lod_mask_from_disk(&inst.geo_hash, &mesh_dir);

            instances_json.push(json!({
                "geo_hash": inst.geo_hash,
                "geo_index": geo_idx,
                "matrix": matrix,
                "name_index": item_name_index,
                "site_name_index": 0,
                "lod_mask": lod_mask,
                "uniforms": {
                    "refno": item.refno.to_string(),
                    "owner_refno": owner_refno_str,
                    "owner_noun": item.noun,
                }
            }));
            total_component_instances += 1;
        }

        ungrouped_json.push(json!({
            "refno": item.refno.to_string(),
            "noun": item.noun,
            "name": item.name,
            "name_index": item_name_index,
            "instances": instances_json,
        }));
    }

    // =========================================================================
    // 7. 组装最终 JSON 并写入文件
    // =========================================================================
    let output_filename = match &root_refno {
        Some(root) => {
            let slug = root
                .to_string()
                .replace(['/', '\\'], "_")
                .replace(' ', "_");
            format!("instances_web_root_{slug}.json")
        }
        None => format!("instances_{dbnum}.json"),
    };

    let mut root_meta = serde_json::Map::new();
    root_meta.insert("version".to_string(), json!(2));
    root_meta.insert("generated_at".to_string(), json!(generated_at));
    if let Some(root) = root_refno {
        root_meta.insert(
            "export_root_refno".to_string(),
            json!(root.to_string()),
        );
    }
    root_meta.insert("names".to_string(), json!(name_table.to_json()));
    root_meta.insert("bran_groups".to_string(), json!(bran_groups_json));
    root_meta.insert("equi_groups".to_string(), json!(equi_groups_json));
    root_meta.insert("ungrouped".to_string(), json!(ungrouped_json));
    let instances_json = serde_json::Value::Object(root_meta);

    let output_path = output_dir.join(&output_filename);
    let json_str = serde_json::to_string_pretty(&instances_json)?;
    std::fs::write(&output_path, &json_str)?;

    if verbose {
        let file_size = std::fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0);
        println!("\n📊 [web] 导出统计:");
        println!("   - BRAN/HANG 分组: {}", bran_groups_json.len());
        println!("   - EQUI 分组: {}", equi_groups_json.len());
        println!("   - 未分组: {}", ungrouped_json.len());
        println!("   - 构件实例: {}", total_component_instances);
        println!("   - TUBI 实例: {}", total_tubing_instances);
        println!("   - 文件大小: {:.2} MB", file_size as f64 / 1024.0 / 1024.0);
        println!("✅ 已写入: {}", output_path.display());
    }

    Ok(WebExportStats {
        bran_group_count: bran_groups_json.len(),
        equi_group_count: equi_groups_json.len(),
        ungrouped_count: ungrouped_json.len(),
        total_component_instances,
        total_tubing_instances,
        elapsed: start_time.elapsed(),
        output_filename,
    })
}
