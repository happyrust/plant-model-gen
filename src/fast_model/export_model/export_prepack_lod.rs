//! Prepack LOD 导出器
//!
//! 生成 Prepack 格式 + 多 LOD 支持：
//! - geometry_shared.glb: 共享几何体（纯数字 geo_hash）
//! - geometry_dedicated.glb: 专用几何体（包含下划线的 geo_hash）
//! - geometry_manifest.json: 几何体清单（包含 LOD 信息）
//! - instances_*.json: 按 zone 分组的实例数据
//! - manifest.json: 总清单

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aios_core::RefnoEnum;
use aios_core::SurrealQueryExt;
use aios_core::mesh_precision::LodLevel;
use aios_core::options::DbOption;
use aios_core::shape::pdms_shape::PlantMesh;
use surrealdb::types::{self as surrealdb_types, SurrealValue};
use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use glam::DMat4;
use parry3d::bounding_volume::Aabb;
use parquet::arrow::ArrowWriter;
use arrow_array::{ArrayRef, Float32Array, RecordBatch, StringArray, UInt32Array};
use arrow_array::builder::{FixedSizeListBuilder, PrimitiveBuilder};
use arrow_array::types::Float32Type;
use arrow_schema::{DataType, Field, Schema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::fast_model::export_model::export_common::{ExportData, TubiRecord, collect_export_data};
use crate::fast_model::export_model::export_unit_mesh_glb::{
    UnitMeshGlbExportResult, UnitMeshGlbExporter, UnitMeshIndexMap,
};
use crate::fast_model::export_model::model_exporter::{
    CommonExportConfig, ExportStats, GlbExportConfig, ModelExporter, collect_export_refnos,
    query_geometry_instances,
};
use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use crate::fast_model::material_config::MaterialLibrary;
use crate::fast_model::query_compat::query_deep_visible_inst_refnos;
use crate::fast_model::query_provider;
use crate::fast_model::unit_converter::{LengthUnit, UnitConverter};

/// LOD 配置
const LOD_LEVELS: &[LodLevel] = &[LodLevel::L1, LodLevel::L2, LodLevel::L3];

/// 几何体类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryType {
    Shared,    // 共享几何体（纯数字）
    Dedicated, // 专用几何体（包含下划线）
}

/// 实例数据文件（按 zone 分组）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstancesData {
    pub version: u32,
    pub generated_at: String,
    pub colors: Vec<[f32; 4]>,
    pub names: Vec<String>,
    pub components: Vec<ComponentGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tubings: Vec<TubingInstance>,
}

#[derive(Debug, Clone)]
struct LodAssetSummary {
    level_tag: String,
    asset_name: String,
    stats: ExportStats,
    mesh_map: UnitMeshIndexMap,
}

/// 构件分组
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentGroup {
    pub refno: String,
    pub noun: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 公共字段（原来在每个 instance 中重复的）
    pub color_index: usize,
    pub name_index: usize,
    pub lod_mask: u32,
    pub spec_value: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uniforms: Option<serde_json::Value>,
    /// 实例列表（只保留真正变化的字段）
    pub instances: Vec<GeoEntry>,
}

/// 层级分组节点（BRAN/EQUI 作为组节点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyGroup {
    /// 组节点的 refno
    pub refno: String,
    /// 组节点类型：BRAN / EQUI
    pub noun: String,
    /// 组节点名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 组节点的 name_index
    pub name_index: usize,
    /// 子构件（非 TUBI）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ComponentGroup>,
    /// 管道实例（按顺序排列的 TUBI）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tubings: Vec<TubingInstance>,
}

/// 几何体实例条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoEntry {
    pub geo_hash: String,
    pub matrix: Vec<f32>,
    pub geo_index: usize,
}

/// 管道实例
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TubingInstance {
    pub refno: String,
    pub noun: String,
    pub geo_hash: String,
    pub matrix: Vec<f32>,
    pub geo_index: usize,
    pub color_index: usize,
    pub name_index: usize,
    pub unit_flag: bool, // 是否为单位 mesh
}

/// 几何体清单
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometryManifest {
    pub version: u32,
    pub generated_at: String,
    pub shared_geometries: Vec<GeometryEntry>,
    pub dedicated_geometries: Vec<GeometryEntry>,
}

/// 几何体条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeometryEntry {
    pub geo_hash: String,
    pub lod_levels: Vec<String>,
    pub file: String,
    pub mesh_index: usize,
}

/// 导出 Prepack LOD 格式（公共接口）
///
/// # 参数
/// - `name_config`: 可选名称配置，用于将三维模型节点名称转换为 PID 对象名称
/// - `export_all_lods`: 是否导出所有 LOD 级别，为 false 时仅导出 L1
/// - `source_length_unit`: 源单位
/// - `target_length_unit`: 目标单位
pub async fn export_prepack_lod_for_refnos(
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    include_descendants: bool,
    filter_nouns: Option<Vec<String>>,
    verbose: bool,
    name_config: Option<&super::name_config::NameConfig>,
    export_all_lods: bool,
    source_length_unit: LengthUnit,
    target_length_unit: LengthUnit,
) -> Result<()> {
    if verbose {
        println!("🚀 开始导出 Prepack LOD 格式...");
        println!("   - 输出目录: {}", output_dir.display());
        println!("   - 单位转换: {} -> {}", source_length_unit.name(), target_length_unit.name());
        if export_all_lods {
            println!("   - 导出所有 LOD 级别: L1, L2, L3");
        } else {
            println!("   - 仅导出 LOD L1 (使用 --export-all-lods 导出所有级别)");
        }
    }

    // 创建输出目录
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    let expanded_refnos = collect_export_refnos(
        refnos,
        include_descendants,
        filter_nouns.as_deref(),
        verbose,
    )
    .await
    .context("收集子孙节点失败")?;

    // P1 修复：SQL 拼接添加分批机制，避免超大数据量时 SQL 过长
    const SQL_BATCH_SIZE: usize = 500;

    // 🏗️ 添加 EQUI 的子组件到 expanded_refnos
    // 确保所有 EQUI 相关的组件都被包含在导出范围内
    let equi_children = {
        if verbose {
            println!("🔍 收集 EQUI 子组件...");
        }

        // 分批查询所有 EQUI 的子组件 refno
        let mut children: Vec<RefnoEnum> = Vec::new();
        for chunk in expanded_refnos.chunks(SQL_BATCH_SIZE) {
            let pe_keys_str = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                r#"
                SELECT VALUE in.id FROM inst_relate
                WHERE owner_type = 'EQUI' AND owner_refno IN [{}]
            "#,
                pe_keys_str
            );

            let batch_children: Vec<RefnoEnum> = aios_core::SUL_DB
                .query_take(sql, 0)
                .await
                .with_context(|| "查询 EQUI 子组件失败")?;
            children.extend(batch_children);
        }

        if verbose {
            println!("   ✅ 找到 {} 个 EQUI 子组件", children.len());
        }

        children
    };

    // 合并 BRAN 子节点和 EQUI 子组件
    let mut all_refnos = expanded_refnos.clone();
    for child in &equi_children {
        if !all_refnos.contains(child) {
            all_refnos.push(*child);
        }
    }

    // P1 修复：移除 all_refnos_pe_keys_str，改用分批查询

    if verbose {
        println!(
            "   📊 最终 refno 总数: {} (BRAN: {} + EQUI: {})",
            all_refnos.len(),
            expanded_refnos.len(),
            equi_children.len()
        );
    }

    let stats_snapshot = ManifestStatsSnapshot {
        refno_count: refnos.len(),
        descendant_count: expanded_refnos.len().saturating_sub(refnos.len()),
        unique_geometries: 0,
        component_instances: 0,
        tubing_instances: 0,
        total_instances: 0,
    };

    // 使用传入的单位转换参数
    let unit_converter = UnitConverter::new(source_length_unit, target_length_unit);
    // manifest 记录采用目标单位避免运行时重复缩放
    let manifest_unit_converter = UnitConverter::new(target_length_unit, target_length_unit);
    let primary_mesh_dir = mesh_dir.to_path_buf();
    let mut base_mesh_dir = mesh_dir.to_path_buf();
    let default_lod_dir = format!("lod_{:?}", db_option.mesh_precision.default_lod);
    while base_mesh_dir
        .file_name()
        .map(|name| name.to_string_lossy() == default_lod_dir)
        .unwrap_or(false)
    {
        if let Some(parent) = base_mesh_dir.parent() {
            base_mesh_dir = parent.to_path_buf();
        } else {
            break;
        }
    }
    if verbose {
        println!("   - Mesh 目录: {}", primary_mesh_dir.display());
        println!("   - 参考号数量: {}", refnos.len());
    }

    let exporter = UnitMeshGlbExporter::default();
    let mut generated_assets: Vec<LodAssetSummary> = Vec::new();
    let filter_cache = filter_nouns.clone();

    // 根据 export_all_lods 参数决定导出哪些 LOD 级别
    let lod_levels_to_export = if export_all_lods {
        LOD_LEVELS
    } else {
        // 仅导出 L1
        &[LodLevel::L1]
    };

    for level in lod_levels_to_export {
        let level_tag = format!("{:?}", level);
        let Some(lod_dir) = resolve_lod_dir(&base_mesh_dir, &level_tag) else {
            println!(
                "⚠️  跳过 LOD {}：目录不存在 {}",
                level_tag,
                base_mesh_dir.join(format!("lod_{level_tag}")).display()
            );
            continue;
        };

        let asset_name = format!("geometry_{}.glb", level_tag);
        let asset_path = output_dir.join(&asset_name);
        let asset_path_str = asset_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("无法转换输出路径: {}", asset_path.display()))?;

        if verbose {
            println!("\n🎯 导出 LOD {} → {}", level_tag, asset_path.display());
        }

        // GLB 中的几何体转换到目标单位，实例矩阵仅做平移转换
        let mut common = CommonExportConfig::with_unit_conversion(
            include_descendants,
            filter_cache.clone(),
            verbose,
            source_length_unit,
            target_length_unit,
        );
        common.use_basic_materials = true;

        let config = GlbExportConfig { common };

        match exporter
            .export(refnos, &lod_dir, asset_path_str, config)
            .await
        {
            Ok(UnitMeshGlbExportResult { stats, mesh_map }) => {
                println!(
                    "   ✅ LOD {} 导出成功：mesh={} (缺失={})，输出大小={} bytes",
                    level_tag,
                    stats.mesh_files_found,
                    stats.mesh_files_missing,
                    stats.output_file_size
                );
                generated_assets.push(LodAssetSummary {
                    level_tag: level_tag.clone(),
                    asset_name,
                    stats,
                    mesh_map,
                });
            }
            Err(err) => {
                eprintln!(
                    "   ❌ LOD {} 导出失败: {} (继续处理其他 LOD)",
                    level_tag, err
                );
            }
        }
    }

    if generated_assets.is_empty() {
        println!("⚠️  没有成功导出的 LOD 资产，请检查 mesh 目录");
        return Ok(());
    }

    let geom_insts = query_geometry_instances(&all_refnos, true, verbose)
        .await
        .context("查询几何体实例失败")?;

    // 🏗️ 分层导出架构：从 inst_relate 查询真正有聚合的 BRAN/HANG owner
    // P1 修复：使用分批查询替代单次大 SQL
    let bran_roots = {
        if verbose {
            println!("🔍 从 inst_relate 查询 BRAN/HANG owner...");
        }

        // 分批查询所有有子节点的 BRAN/HANG owner
        let mut bran_hang_owners: Vec<RefnoEnum> = Vec::new();
        for chunk in all_refnos.chunks(SQL_BATCH_SIZE) {
            let pe_keys_str = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                r#"
                SELECT VALUE owner_refno FROM inst_relate
                WHERE owner_type in ['BRAN', 'HANG'] AND owner_refno IN [{}]
            "#,
                pe_keys_str
            );

            let batch_owners: Vec<RefnoEnum> = aios_core::SUL_DB
                .query_take(sql, 0)
                .await
                .with_context(|| "查询 BRAN/HANG owner 失败")?;
            bran_hang_owners.extend(batch_owners);
        }

        // 在 Rust 中去重，避免 SurrealDB 复杂的数组操作
        bran_hang_owners.sort();
        bran_hang_owners.dedup();

        if verbose {
            println!(
                "   ✅ 找到 {} 个有子节点的 BRAN/HANG owner",
                bran_hang_owners.len()
            );
        }

        bran_hang_owners
    };

    // 🏗️ 从 inst_relate 查询真正有聚合的 EQUI owner
    // P1 修复：使用分批查询替代单次大 SQL
    let equi_owners = {
        if verbose {
            println!("🔍 从 inst_relate 查询 EQUI owner...");
        }

        // 分批查询所有有子节点的 EQUI owner
        let mut equi_owner_list: Vec<RefnoEnum> = Vec::new();
        for chunk in all_refnos.chunks(SQL_BATCH_SIZE) {
            let pe_keys_str = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                r#"
                SELECT VALUE owner_refno FROM inst_relate
                WHERE owner_type = 'EQUI' AND owner_refno IN [{}]
            "#,
                pe_keys_str
            );

            let batch_owners: Vec<RefnoEnum> = aios_core::SUL_DB
                .query_take(sql, 0)
                .await
                .with_context(|| "查询 EQUI owner 失败")?;
            equi_owner_list.extend(batch_owners);
        }

        // 在 Rust 中去重
        equi_owner_list.sort();
        equi_owner_list.dedup();

        if verbose {
            println!(
                "   ✅ 找到 {} 个有子节点的 EQUI owner",
                equi_owner_list.len()
            );
        }

        equi_owner_list
    };

    let export_data = collect_export_data(
        geom_insts,
        &all_refnos,
        &primary_mesh_dir,
        verbose,
        Some(&bran_roots),
    )
    .await
    .context("收集导出数据失败")?;

    // 为组件准备名称映射（使用 full name）
    // 包括 all_refnos、bran_roots、equi_owners 和所有 component 的 owner_refno
    let mut refno_name_map: HashMap<RefnoEnum, String> = HashMap::new();
    
    // 收集所有需要查询 full name 的 refno
    let mut all_name_refnos: Vec<RefnoEnum> = all_refnos.clone();
    all_name_refnos.extend(bran_roots.iter().copied());
    all_name_refnos.extend(equi_owners.iter().copied());
    // 添加所有 component 的 owner_refno（确保 BRAN/EQUI owner 的 name 不为 null）
    for component in &export_data.components {
        if let Some(owner_refno) = component.owner_refno {
            all_name_refnos.push(owner_refno);
        }
    }
    all_name_refnos.sort();
    all_name_refnos.dedup();
    
    // P0 修复：使用批量查询替代循环单查，避免 N+1 查询问题
    // 分批查询以避免 SQL 过长
    const NAME_BATCH_SIZE: usize = 500;
    for chunk in all_name_refnos.chunks(NAME_BATCH_SIZE) {
        match aios_core::rs_surreal::query_full_names_map(chunk).await {
            Ok(names_map) => {
                for (refno, full_name) in names_map {
                    if !full_name.is_empty() {
                        // 只去掉开头的斜线，保持其他字符原样
                        let trimmed_name = full_name.trim().trim_start_matches('/').to_string();
                        if !trimmed_name.is_empty() {
                            // 如果有名称配置，使用配置转换名称；否则保持原样
                            let final_name = if let Some(config) = name_config {
                                config.convert_name(&trimmed_name)
                            } else {
                                trimmed_name
                            };
                            refno_name_map.insert(refno, final_name);
                        }
                    }
                }
            }
            Err(e) => {
                if verbose {
                    eprintln!("⚠️ 批量查询名称失败: {}", e);
                }
            }
        }
    }

    if export_data.total_instances == 0 {
        println!("⚠️  未找到任何几何体实例，跳过 manifest 生成");
        return Ok(());
    }

    // 加载材质与配色信息（严格按照 ColorSchemes.toml / 默认方案）
    let material_library =
        MaterialLibrary::load_default().context("加载默认材质库失败（用于颜色配置）")?;

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    // 使用 generated_assets 中的 mesh_map 构建 geo_index_map，而不是从空的 unique_geometries
    let (geo_hashes, geo_index_map) = if !generated_assets.is_empty() {
        // 使用第一个 LOD 的 mesh_map
        let mesh_map = &generated_assets[0].mesh_map.0;
        let mut hashes: Vec<String> = mesh_map.keys().cloned().collect();
        hashes.sort();
        let index_map: HashMap<String, usize> = hashes.iter().enumerate().map(|(i, h)| (h.clone(), i)).collect();
        (hashes, index_map)
    } else {
        build_geo_index_map(&export_data)
    };
    let geo_nouns = collect_geo_nouns(&export_data);
    let geometry_entries = build_geometry_entries(
        &geo_hashes,
        &geo_index_map,
        &geo_nouns,
        &generated_assets,
        &std::collections::HashMap::new(),
        &unit_converter,
    );

    let (instances_json, component_instance_count) = build_instances_payload(
        &export_data,
        &geo_index_map,
        &generated_at,
        &generated_assets,
        &unit_converter,
        &material_library,
        &refno_name_map,
        &equi_owners,
        verbose,
    );

    let mut manifest_stats = stats_snapshot;
    manifest_stats.unique_geometries = geo_hashes.len();
    manifest_stats.component_instances = component_instance_count;
    manifest_stats.tubing_instances = export_data.tubi_count;
    manifest_stats.total_instances = export_data.total_instances;

    write_bundle_manifests(
        output_dir,
        &generated_assets,
        &manifest_stats,
        geometry_entries,
        instances_json,
        &generated_at,
        &manifest_unit_converter,
    )?;

    if verbose {
        println!("\n📦 已生成的 LOD 资产:");
        for record in &generated_assets {
            println!(
                "   - LOD {} → {} (mesh_found={}, mesh_missing={})",
                record.level_tag,
                record.asset_name,
                record.stats.mesh_files_found,
                record.stats.mesh_files_missing
            );
        }
        println!("   - manifest.json / geometry_manifest.json / instances.json 已写入");
    }

    Ok(())
}

#[derive(Default, Clone)]
struct ManifestStatsSnapshot {
    refno_count: usize,
    descendant_count: usize,
    unique_geometries: usize,
    component_instances: usize,
    tubing_instances: usize,
    total_instances: usize,
}

fn write_bundle_manifests(
    output_dir: &Path,
    generated_assets: &[LodAssetSummary],
    stats_snapshot: &ManifestStatsSnapshot,
    geometry_entries: Vec<serde_json::Value>,
    instances_json: serde_json::Value,
    generated_at: &str,
    unit_converter: &UnitConverter,
) -> Result<()> {
    let geometry_manifest_name = "geometry_manifest.json";
    let instance_manifest_name = "instances.json";

    let geometry_manifest = json!({
        "version": 1,
        "generated_at": generated_at,
        "coordinate_system": {
            "handedness": "right",
            "up_axis": "Y",
        },
        "geometries": geometry_entries,
    });

    let geometry_manifest_bytes = serde_json::to_vec_pretty(&geometry_manifest)?;
    fs::write(
        output_dir.join(geometry_manifest_name),
        &geometry_manifest_bytes,
    )?;

    let instances_bytes = serde_json::to_vec_pretty(&instances_json)?;
    fs::write(output_dir.join(instance_manifest_name), &instances_bytes)?;

    let geometry_manifest_ref = json!({
        "path": geometry_manifest_name,
        "bytes": geometry_manifest_bytes.len() as u64,
        "sha256": sha256_from_bytes(&geometry_manifest_bytes),
    });

    let instance_manifest_ref = json!({
        "path": instance_manifest_name,
        "bytes": instances_bytes.len() as u64,
        "sha256": sha256_from_bytes(&instances_bytes),
    });

    let mut geometry_assets = serde_json::Map::new();
    let mut lod_profiles = Vec::new();

    for (idx, summary) in generated_assets.iter().enumerate() {
        let numeric_level = summary
            .level_tag
            .trim_start_matches('L')
            .parse::<u32>()
            .unwrap_or(idx as u32 + 1);

        let asset_path = output_dir.join(&summary.asset_name);
        let metadata = fs::metadata(&asset_path)
            .with_context(|| format!("读取文件元数据失败: {}", asset_path.display()))?;
        let sha256 = sha256_for_file(&asset_path)?;

        geometry_assets.insert(
            summary.level_tag.clone(),
            json!({
                "path": summary.asset_name,
                "bytes": metadata.len(),
                "sha256": sha256,
                "mesh_files_found": summary.stats.mesh_files_found,
                "mesh_files_missing": summary.stats.mesh_files_missing
            }),
        );

        lod_profiles.push(json!({
            "level": numeric_level,
            "asset_key": summary.level_tag,
            "priority": idx as u32,
            "target_triangles": 0,
            "max_position_error": 0.0,
            "default_material": default_material_for_level(numeric_level),
        }));
    }

    let manifest = json!({
        "version": "1.1.0",
        "generated_at": generated_at,
        "files": {
            "geometry_manifest": geometry_manifest_ref,
            "instance_manifest": instance_manifest_ref,
            "geometry_assets": geometry_assets,
        },
        "unit_conversion": json!({
            "source_unit": unit_converter.source_unit.name(),
            "target_unit": unit_converter.target_unit.name(),
            "factor": unit_converter.conversion_factor(),
            "precision": 6,
        }),
        "lod_profiles": lod_profiles,
        "stats": {
            "refno_count": stats_snapshot.refno_count,
            "descendant_count": stats_snapshot.descendant_count,
            "unique_geometries": stats_snapshot.unique_geometries,
            "component_instances": stats_snapshot.component_instances,
            "tubing_instances": stats_snapshot.tubing_instances,
            "total_instances": stats_snapshot.total_instances,
            "export_duration_ms": 0,
        }
    });

    fs::write(
        output_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    Ok(())
}

fn resolve_lod_dir(base: &Path, level_tag: &str) -> Option<PathBuf> {
    let primary = base.join(format!("lod_{level_tag}"));
    if primary.is_dir() {
        let nested = primary.join(format!("lod_{level_tag}"));
        if nested.is_dir() {
            Some(nested)
        } else {
            Some(primary)
        }
    } else {
        None
    }
}

fn build_geo_index_map(export_data: &ExportData) -> (Vec<String>, HashMap<String, usize>) {
    // 只包含成功加载的几何体，排除加载失败的 TUBI
    let mut geo_hashes: Vec<String> = export_data.valid_geo_hashes.iter().cloned().collect();
    geo_hashes.sort();
    let mut index_map = HashMap::new();
    for (idx, hash) in geo_hashes.iter().enumerate() {
        index_map.insert(hash.clone(), idx);
    }
    (geo_hashes, index_map)
}

fn collect_geo_nouns(export_data: &ExportData) -> HashMap<String, Vec<String>> {
    let mut noun_map: HashMap<String, HashSet<String>> = HashMap::new();

    for component in &export_data.components {
        for geometry in &component.geometries {
            noun_map
                .entry(geometry.geo_hash.clone())
                .or_default()
                .insert(component.noun.clone());
        }
    }

    for tubing in &export_data.tubings {
        noun_map
            .entry(tubing.geo_hash.clone())
            .or_default()
            .insert("TUBING".to_string());
    }

    noun_map
        .into_iter()
        .map(|(hash, set)| {
            let mut nouns: Vec<String> = set.into_iter().collect();
            nouns.sort();
            (hash, nouns)
        })
        .collect()
}

fn build_geometry_entries(
    geo_hashes: &[String],
    geo_index_map: &HashMap<String, usize>,
    geo_nouns: &HashMap<String, Vec<String>>,
    lod_assets: &[LodAssetSummary],
    unique_geometries: &HashMap<String, Arc<PlantMesh>>,
    unit_converter: &UnitConverter,
) -> Vec<serde_json::Value> {
    geo_hashes
        .iter()
        .map(|geo_hash| {
            let metrics = unique_geometries
                .get(geo_hash)
                .map(|mesh| extract_geometry_metrics(mesh, unit_converter))
                .unwrap_or_default();

            let mut lods = Vec::new();
            for (idx, summary) in lod_assets.iter().enumerate() {
                if let Some(mesh_index) = summary.mesh_map.get(geo_hash) {
                    let numeric_level = summary
                        .level_tag
                        .trim_start_matches('L')
                        .parse::<u32>()
                        .unwrap_or(idx as u32 + 1);
                    lods.push(json!({
                        "level": numeric_level,
                        "asset_key": summary.level_tag,
                        "mesh_index": mesh_index,
                        "node_index": mesh_index,
                        "triangle_count": metrics.triangle_count,
                        "error_metric": 0.0
                    }));
                }
            }

            let bounding_box = metrics
                .bounding_box
                .as_ref()
                .map(|(min, max)| json!({ "min": min, "max": max }));
            let bounding_sphere = metrics
                .bounding_sphere
                .as_ref()
                .map(|(center, radius)| json!({ "center": center, "radius": radius }));

            json!({
                "geo_hash": geo_hash,
                "geo_index": geo_index_map.get(geo_hash).copied().unwrap_or(0),
                "nouns": geo_nouns.get(geo_hash).cloned().unwrap_or_default(),
                "vertex_count": metrics.vertex_count,
                "triangle_count": metrics.triangle_count,
                "bounding_box": bounding_box.unwrap_or(serde_json::Value::Null),
                "bounding_sphere": bounding_sphere.unwrap_or(serde_json::Value::Null),
                "lods": lods,
            })
        })
        .collect()
}

#[derive(Default)]
struct GeometryMetrics {
    vertex_count: usize,
    triangle_count: usize,
    bounding_box: Option<([f32; 3], [f32; 3])>,
    bounding_sphere: Option<([f32; 3], f32)>,
}

fn extract_geometry_metrics(mesh: &PlantMesh, unit_converter: &UnitConverter) -> GeometryMetrics {
    let vertex_count = mesh.vertices.len();
    let triangle_count = mesh.indices.len() / 3;
    let bounds = mesh.aabb.clone().or_else(|| mesh.cal_aabb());

    let bounding_box = bounds.as_ref().map(|aabb| {
        let mut min = [aabb.mins.x, aabb.mins.y, aabb.mins.z];
        let mut max = [aabb.maxs.x, aabb.maxs.y, aabb.maxs.z];
        if unit_converter.needs_conversion() {
            for v in min.iter_mut().chain(max.iter_mut()) {
                *v = unit_converter.convert_value(*v);
            }
        }
        (min, max)
    });

    let bounding_sphere = bounds.as_ref().map(|aabb| {
        let mut center = [
            (aabb.mins.x + aabb.maxs.x) * 0.5,
            (aabb.mins.y + aabb.maxs.y) * 0.5,
            (aabb.mins.z + aabb.maxs.z) * 0.5,
        ];
        let dx = aabb.maxs.x - aabb.mins.x;
        let dy = aabb.maxs.y - aabb.mins.y;
        let dz = aabb.maxs.z - aabb.mins.z;
        let mut radius = 0.5 * (dx * dx + dy * dy + dz * dz).sqrt();
        if unit_converter.needs_conversion() {
            for v in center.iter_mut() {
                *v = unit_converter.convert_value(*v);
            }
            radius = unit_converter.convert_value(radius);
        }
        (center, radius)
    });

    GeometryMetrics {
        vertex_count,
        triangle_count,
        bounding_box,
        bounding_sphere,
    }
}

fn build_instances_payload(
    export_data: &ExportData,
    geo_index_map: &HashMap<String, usize>,
    generated_at: &str,
    lod_assets: &[LodAssetSummary],
    unit_converter: &UnitConverter,
    material_library: &MaterialLibrary,
    refno_name_map: &HashMap<RefnoEnum, String>,
    equi_owners: &[RefnoEnum],
    verbose: bool,
) -> (serde_json::Value, usize) {
    let mut color_palette = ColorPalette::new(material_library);
    let mut component_instance_count = 0usize;

    // ========== 第一步：按 BRAN 分组 ==========
    // 收集所有 BRAN owner 的 refno
    let mut bran_owners: HashSet<RefnoEnum> = HashSet::new();
    for component in &export_data.components {
        if matches!(component.owner_noun.as_deref(), Some("BRAN") | Some("HANG")) {
            if let Some(owner) = component.owner_refno {
                bran_owners.insert(owner);
            }
        }
    }
    // 只将真正的 BRAN 组件作为 top-level 组，不包含 TUBI 的 owner
    // TUBI 应该关联到其真正的 BRAN owner，而不是创建新的组

    // 按 BRAN owner 分组构件
    let mut bran_children_map: HashMap<
        RefnoEnum,
        Vec<&crate::fast_model::export_model::export_common::ComponentRecord>,
    > = HashMap::new();
    for component in &export_data.components {
        if matches!(component.owner_noun.as_deref(), Some("BRAN") | Some("HANG")) {
            if let Some(owner) = component.owner_refno {
                bran_children_map.entry(owner).or_default().push(component);
            }
        }
    }

    // 创建子组件到 BRAN 的反向映射
    let mut child_to_bran: HashMap<RefnoEnum, RefnoEnum> = HashMap::new();
    for (bran_refno, children) in &bran_children_map {
        for child in children {
            child_to_bran.insert(child.refno, *bran_refno);
        }
    }

    // 按 BRAN owner 分组 TUBI（保持顺序）
    let mut bran_tubi_map: BTreeMap<RefnoEnum, Vec<&TubiRecord>> = BTreeMap::new();
    for tubing in &export_data.tubings {
        let key_refno = if tubing.owner_refno.is_unset() {
            tubing.refno
        } else {
            // 查找最终的 BRAN owner
            let mut current_refno = tubing.owner_refno;
            let mut final_bran = tubing.owner_refno;

            // 递归查找直到找到 BRAN owner
            while let Some(&bran_owner) = child_to_bran.get(&current_refno) {
                final_bran = bran_owner;
                current_refno = bran_owner;
            }

            final_bran
        };

        bran_tubi_map.entry(key_refno).or_default().push(tubing);
    }

    // 构建 BRAN 层级分组
    let mut bran_groups: Vec<serde_json::Value> = Vec::new();
    let mut bran_owners_sorted: Vec<_> = bran_owners.iter().copied().collect();
    bran_owners_sorted.sort();

    for bran_refno in bran_owners_sorted {
        let bran_name = refno_name_map.get(&bran_refno).cloned();

        // 构建子构件列表
        let mut children_entries: Vec<serde_json::Value> = Vec::new();
        if let Some(children) = bran_children_map.get(&bran_refno) {
            for component in children {
                // 使用 refno_name_map 中的 full name
                let component_name = refno_name_map.get(&component.refno).cloned();
                let color_index = color_palette.index_for_noun(&component.noun);

                let mut instances = Vec::new();
                for geom in &component.geometries {
                    if let Some(&geo_index) = geo_index_map.get(&geom.geo_hash) {
                        component_instance_count += 1;

                        // 调试：输出 unit_flag 状态（仅纯数字 geo_hash）
                        if verbose && !geom.geo_hash.contains('_') {
                            let eff_flag = effective_unit_flag(&geom.geo_hash, geom.unit_flag);
                            println!("   🔍 [BRAN] geo_hash={} unit_flag={} effective={}", geom.geo_hash, geom.unit_flag, eff_flag);
                        }

                        instances.push(json!({
                            "geo_hash": geom.geo_hash.clone(),
                            "geo_index": geo_index,
                            "geo_transform": mat4_to_vec(&geom.local_transform, unit_converter, effective_unit_flag(&geom.geo_hash, geom.unit_flag)),
                        }));
                    }
                }

                if !instances.is_empty() {
                    let lod_mask = compute_lod_mask(&component.geometries[0].geo_hash, lod_assets);
                    
                    children_entries.push(json!({
                        "refno": component.refno.to_string(),
                        "noun": component.noun,
                        "name": component_name,
                        "color_index": color_index,
                        "lod_mask": lod_mask,
                        "spec_value": component.spec_value,
                        "refno_transform": mat4_to_vec(&component.world_transform, unit_converter, false),
                        "instances": instances,
                    }));
                }
            }
        }

        // 构建 TUBI 列表（按顺序）
        let mut tubi_entries: Vec<serde_json::Value> = Vec::new();
        if let Some(tubings) = bran_tubi_map.get(&bran_refno) {
            let color_index = color_palette.index_for_noun("TUBI");

            for (tubi_order, tubing) in tubings.iter().enumerate() {
                if let Some(&geo_index) = geo_index_map.get(&tubing.geo_hash) {
                    // 使用 refno_name_map 中的 full name
                    let tubi_name = refno_name_map.get(&tubing.refno).cloned();
                    let lod_mask = compute_lod_mask(&tubing.geo_hash, lod_assets);

                    tubi_entries.push(json!({
                        "refno": tubing.refno.to_string(),
                        "noun": "TUBI",
                        "name": tubi_name,
                        "geo_hash": tubing.geo_hash,
                        "geo_index": geo_index,
                        "matrix": mat4_to_vec(&tubing.transform, unit_converter, true), // TUBI 统一是 unit_mesh
                        "color_index": color_index,
                        "order": tubi_order,
                        "lod_mask": lod_mask,
                        "spec_value": tubing.spec_value,
                    }));
                }
            }
        }

        bran_groups.push(json!({
            "refno": bran_refno.to_string(),
            "noun": "BRAN",
            "name": bran_name,
            "children": children_entries,
            "tubings": tubi_entries,
        }));
    }

    // ========== 第二步：按 EQUI 分组 ==========
    // 🏗️ 使用从 inst_relate 查询的 EQUI owner，而不是重新过滤
    let mut equi_children_map: HashMap<
        RefnoEnum,
        Vec<&crate::fast_model::export_model::export_common::ComponentRecord>,
    > = HashMap::new();

    if verbose {
        println!("🔍 调试 EQUI 数据匹配...");
        println!("   - EQUI owners 列表: {:?}", equi_owners);
        println!("   - 总 components 数量: {}", export_data.components.len());

        // 统计所有 owner_refno 为 EQUI 的 components
        let mut equi_components = 0;
        for component in &export_data.components {
            if matches!(component.owner_noun.as_deref(), Some("EQUI")) {
                equi_components += 1;
                if equi_components <= 5 {
                    println!(
                        "   - EQUI component[{}]: owner_refno={:?}, refno={}",
                        equi_components, component.owner_refno, component.refno
                    );
                }
            }
        }
        println!("   - 总 EQUI components 数量: {}", equi_components);
    }

    for component in &export_data.components {
        if let Some(owner) = component.owner_refno {
            if equi_owners.contains(&owner) {
                if verbose {
                    println!(
                        "   ✅ 匹配成功: owner={:?} -> component refno={}",
                        owner, component.refno
                    );
                }
                equi_children_map.entry(owner).or_default().push(component);
            }
        }
    }

    if verbose {
        println!("   - EQUI children_map 大小: {}", equi_children_map.len());
        for (owner, children) in &equi_children_map {
            println!("   - owner {:?} 有 {} 个子构件", owner, children.len());
        }
    }

    // 构建 EQUI 层级分组
    let mut equi_groups: Vec<serde_json::Value> = Vec::new();
    let mut equi_owners_sorted = equi_owners.to_vec();
    equi_owners_sorted.sort();

    for equi_refno in &equi_owners_sorted {
        let equi_name = refno_name_map.get(&equi_refno).cloned();

        // 构建子构件列表
        let mut children_entries: Vec<serde_json::Value> = Vec::new();
        if let Some(children) = equi_children_map.get(&equi_refno) {
            for component in children {
                // 使用 refno_name_map 中的 full name
                let component_name = refno_name_map.get(&component.refno).cloned();
                let color_index = color_palette.index_for_noun(&component.noun);

                let mut instances = Vec::new();
                for geom in &component.geometries {
                    if let Some(&geo_index) = geo_index_map.get(&geom.geo_hash) {
                        component_instance_count += 1;
                        
                        instances.push(json!({
                            "geo_hash": geom.geo_hash.clone(),
                            "geo_index": geo_index,
                            "geo_transform": mat4_to_vec(&geom.local_transform, unit_converter, effective_unit_flag(&geom.geo_hash, geom.unit_flag)),
                        }));
                    }
                }

                if !instances.is_empty() {
                    let lod_mask = compute_lod_mask(&component.geometries[0].geo_hash, lod_assets);
                    
                    children_entries.push(json!({
                        "refno": component.refno.to_string(),
                        "noun": component.noun,
                        "name": component_name,
                        "color_index": color_index,
                        "lod_mask": lod_mask,
                        "spec_value": component.spec_value,
                        "refno_transform": mat4_to_vec(&component.world_transform, unit_converter, false),
                        "instances": instances,
                    }));
                }
            }
        }

        equi_groups.push(json!({
            "refno": equi_refno.to_string(),
            "noun": "EQUI",
            "name": equi_name,
            "children": children_entries,
        }));
    }

    // ========== 第三步：收集未分组的构件 ==========
    // 🏗️ 使用 NOT IN 排除已处理的 BRAN/HANG/EQUI owner_refno
    let processed_owners: HashSet<RefnoEnum> = {
        let mut set = HashSet::new();
        // 添加已处理的 BRAN/HANG owner (从 bran_groups 中提取)
        for bran_group in &bran_groups {
            if let Some(refno_str) = bran_group.get("refno").and_then(|v| v.as_str()) {
                if let Ok(refno) = refno_str.parse::<RefnoEnum>() {
                    set.insert(refno);
                }
            }
        }
        // 添加已处理的 EQUI owner
        for equi_refno in &equi_owners_sorted {
            set.insert(*equi_refno);
        }
        set
    };

    let mut ungrouped_entries: Vec<serde_json::Value> = Vec::new();
    for component in &export_data.components {
        // 🎯 使用 NOT IN 逻辑排除已处理的 owner_refno
        if let Some(owner_refno) = component.owner_refno {
            if processed_owners.contains(&owner_refno) {
                continue; // 跳过已经在 BRAN/HANG/EQUI 分组中的构件
            }
        }

        // 使用 refno_name_map 中的 full name
        let component_name = refno_name_map.get(&component.refno).cloned();
        let color_index = color_palette.index_for_noun(&component.noun);

        let mut instances = Vec::new();
        for geom in &component.geometries {
            if let Some(&geo_index) = geo_index_map.get(&geom.geo_hash) {
                component_instance_count += 1;
                
                instances.push(json!({
                    "geo_hash": geom.geo_hash.clone(),
                    "geo_index": geo_index,
                    "geo_transform": mat4_to_vec(&geom.local_transform, unit_converter, effective_unit_flag(&geom.geo_hash, geom.unit_flag)),
                }));
            }
        }

        if !instances.is_empty() {
            let lod_mask = compute_lod_mask(&component.geometries[0].geo_hash, lod_assets);
            
            ungrouped_entries.push(json!({
                "refno": component.refno.to_string(),
                "noun": component.noun,
                "name": component_name,
                "color_index": color_index,
                "lod_mask": lod_mask,
                "spec_value": component.spec_value,
                "refno_transform": mat4_to_vec(&component.world_transform, unit_converter, false),
                "instances": instances,
            }));
        }
    }

    let instances_json = json!({
        "version": 2,
        "generated_at": generated_at,
        "colors": color_palette.into_colors(),
        "bran_groups": bran_groups,
        "equi_groups": equi_groups,
        "ungrouped": ungrouped_entries,
    });

    (instances_json, component_instance_count)
}

fn compute_lod_mask(geo_hash: &str, lod_assets: &[LodAssetSummary]) -> u32 {
    let mut mask = 0u32;
    for summary in lod_assets {
        if summary.mesh_map.get(geo_hash).is_some() {
            if let Ok(level) = summary.level_tag.trim_start_matches('L').parse::<u32>() {
                if (1..=32).contains(&level) {
                    mask |= 1 << (level - 1);
                }
            }
        }
    }

    if mask == 0 {
        let levels = lod_assets.len().min(32);
        if levels > 0 {
            mask = (1u32 << levels) - 1;
        }
    }

    if mask == 0 {
        mask = 0b111;
    }

    mask
}

/// 判断 geo_hash 是否为标准单位几何体（0, 1, 2, 3 等小数字）
/// 这些是预定义的单位网格，应该强制 unit_flag=true
fn is_standard_unit_geometry(geo_hash: &str) -> bool {
    // 尝试解析为数字，如果是 0-9 的小数字，则认为是标准单位几何体
    if let Ok(num) = geo_hash.parse::<u64>() {
        num < 10
    } else {
        false
    }
}

/// 获取有效的 unit_flag，对标准单位几何体强制返回 true
fn effective_unit_flag(geo_hash: &str, original_flag: bool) -> bool {
    if is_standard_unit_geometry(geo_hash) {
        true
    } else {
        original_flag
    }
}

fn mat4_to_vec(matrix: &DMat4, unit_converter: &UnitConverter, unit_flag: bool) -> Vec<f32> {
    let mut cols = matrix.to_cols_array();
    if unit_converter.needs_conversion() {
        let factor = unit_converter.conversion_factor() as f64;
        // Unit mesh：缩放旋转/缩放部分；普通 mesh：不缩放旋转/缩放部分（已在顶点上）
        if unit_flag {
            // 缩放旋转部分（前3列）
            for i in 0..3 {
                cols[i] *= factor; // 第一列
                cols[4 + i] *= factor; // 第二列
                cols[8 + i] *= factor; // 第三列
            }
        }
        // 平移部分始终需要缩放（世界坐标必须单位转换）
        cols[12] *= factor;
        cols[13] *= factor;
        cols[14] *= factor;
    }
    cols.iter().map(|v| *v as f32).collect()
}

#[derive(Default)]
struct NameTable {
    entries: Vec<serde_json::Value>,
    index_map: HashMap<(String, String), usize>,
}

impl NameTable {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            index_map: HashMap::new(),
        }
    }

    fn get_or_insert(&mut self, kind: &str, value: &str) -> usize {
        let key = (kind.to_string(), value.to_string());
        if let Some(idx) = self.index_map.get(&key) {
            *idx
        } else {
            let idx = self.entries.len();
            self.entries.push(json!({ "kind": kind, "value": value }));
            self.index_map.insert(key, idx);
            idx
        }
    }

    fn into_entries(self) -> Vec<serde_json::Value> {
        self.entries
    }
}

struct ColorPalette<'a> {
    colors: Vec<[f32; 4]>,
    index_map: HashMap<String, usize>,
    material_library: &'a MaterialLibrary,
}

impl<'a> ColorPalette<'a> {
    fn new(material_library: &'a MaterialLibrary) -> Self {
        Self {
            colors: Vec::new(),
            index_map: HashMap::new(),
            material_library,
        }
    }

    fn index_for_noun(&mut self, noun: &str) -> usize {
        let key = noun.to_ascii_uppercase();
        if let Some(idx) = self.index_map.get(&key) {
            return *idx;
        }

        let color = self.color_for_noun(&key);
        let idx = self.colors.len();
        self.colors.push(color);
        self.index_map.insert(key, idx);
        idx
    }

    fn color_at(&self, index: usize) -> Option<[f32; 4]> {
        self.colors.get(index).cloned()
    }

    fn into_colors(mut self) -> Vec<[f32; 4]> {
        if self.colors.is_empty() {
            self.colors.push([0.82, 0.83, 0.84, 1.0]);
        }
        self.colors
    }

    /// 严格依据 ColorSchemes.toml / 默认方案获取颜色；
    /// 若配色表中不存在该类型，则使用内置的完整颜色映射表。
    fn color_for_noun(&self, noun: &str) -> [f32; 4] {
        // 首先尝试从 MaterialLibrary（ColorSchemeManager）获取
        if let Some(c) = self.material_library.get_normalized_color_for_noun(noun) {
            return c;
        }

        // 内置的完整颜色映射表（与 rs-core/rs-plant3-d 保持一致）
        // 颜色值为 RGBA [0-255]，转换为归一化 [0.0-1.0]
        let color_u8: [u8; 4] = match noun.to_uppercase().as_str() {
            // 标准 PDMS 类型
            "UNKOWN" => [192, 192, 192, 255],
            "CE" => [0, 100, 200, 180],
            "EQUI" => [255, 190, 0, 255],
            "PIPE" => [255, 255, 0, 255],
            "HANG" => [255, 126, 0, 255],
            "STRU" => [0, 150, 255, 255],
            "SCTN" => [188, 141, 125, 255],
            "GENSEC" => [188, 141, 125, 255],
            "WALL" => [150, 150, 150, 255],
            "STWALL" => [150, 150, 150, 255],
            "CWALL" => [120, 120, 120, 255],
            "GWALL" => [173, 216, 230, 128],
            "FLOOR" => [210, 180, 140, 255],
            "CFLOOR" => [160, 130, 100, 255],
            "PANE" => [220, 220, 220, 255],
            "ROOM" => [144, 238, 144, 100],
            "AREADEF" => [221, 160, 221, 80],
            "HVAC" => [175, 238, 238, 255],
            "EXTR" => [147, 112, 219, 255],
            "REVO" => [138, 43, 226, 255],
            "HANDRA" => [255, 215, 0, 255],
            "CWBRAN" => [255, 140, 0, 255],
            "CTWALL" => [176, 196, 222, 150],
            "DEMOPA" => [255, 69, 0, 255],
            "INSURQ" => [255, 182, 193, 255],
            "STRLNG" => [0, 255, 255, 255],

            // 管道相关类型（继承 PIPE 颜色）
            "BRAN" => [255, 255, 0, 255],      // 分支 - 黄色
            "TUBI" => [255, 255, 0, 255],      // 管道段 - 黄色
            "VALV" => [255, 100, 100, 255],    // 阀门 - 浅红色
            "INST" => [100, 200, 255, 255],    // 仪表 - 浅蓝色
            "ATTA" => [200, 200, 100, 255],    // 附件 - 黄绿色

            // 变换/几何类型
            "TRNS" => [192, 192, 192, 255],    // 变换 - 灰色
            "TMPL" => [180, 180, 180, 255],    // 模板 - 灰色
            "SUBE" => [255, 190, 0, 255],      // 子设备 - 橙黄色（同 EQUI）
            "NOZZ" => [255, 160, 0, 255],      // 喷嘴 - 橙色

            // 结构相关
            "FRMW" => [0, 150, 255, 255],      // 框架 - 蓝色（同 STRU）
            "SBFR" => [0, 150, 255, 255],      // 子框架 - 蓝色
            "STSE" => [188, 141, 125, 255],    // 结构截面 - 棕色（同 SCTN）
            "JOIN" => [100, 100, 200, 255],    // 连接件 - 紫蓝色
            "SJOI" => [100, 100, 200, 255],    // 结构连接 - 紫蓝色
            "PNOD" => [150, 150, 200, 255],    // 节点 - 浅紫色

            // 电气/电缆
            "CWAY" => [255, 165, 0, 255],      // 电缆桥架 - 橙色
            "CTRAY" => [255, 165, 0, 255],     // 电缆托盘 - 橙色

            // 默认颜色（灰色）
            _ => [192, 192, 192, 255],
        };

        // 转换为归一化颜色
        [
            color_u8[0] as f32 / 255.0,
            color_u8[1] as f32 / 255.0,
            color_u8[2] as f32 / 255.0,
            color_u8[3] as f32 / 255.0,
        ]
    }
}

fn sha256_for_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("打开文件失败: {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("读取文件失败: {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_from_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn default_material_for_level(level: u32) -> &'static str {
    match level {
        1 => "pbrStandard",
        2 => "litLambert",
        _ => "flatColor",
    }
}

async fn collect_deep_visible_refnos(
    refnos: &[RefnoEnum],
    verbose: bool,
    filter_nouns: Option<&[String]>,
) -> Result<Vec<RefnoEnum>> {
    let mut collected = HashSet::new();
    for &refno in refnos {
        match query_deep_visible_inst_refnos(refno).await {
            Ok(visible) => {
                if visible.is_empty() {
                    collected.insert(refno);
                } else {
                    collected.extend(visible);
                }
            }
            Err(err) => {
                if verbose {
                    eprintln!("⚠️ 查询深度可见实例失败 (refno: {}): {}", refno, err);
                }
                collected.insert(refno);
            }
        }
    }

    let mut result: Vec<RefnoEnum> = collected.into_iter().collect();
    result.sort();

    if let Some(filter) = filter_nouns {
        let filter_set: HashSet<String> =
            filter.iter().map(|t| t.to_uppercase()).collect();
        if !filter_set.is_empty() {
            match query_provider::get_pes_batch(&result).await {
                Ok(pes) => {
                    let mut filtered = Vec::new();
                    for pe in pes {
                        if filter_set.contains(&pe.noun.to_uppercase()) {
                            filtered.push(pe.refno);
                        }
                    }
                    filtered.sort();
                    return Ok(filtered);
                }
                Err(err) => {
                    if verbose {
                        eprintln!("⚠️ 过滤可见实例失败，已回退到未过滤列表: {}", err);
                    }
                }
            }
        }
    }

    Ok(result)
}

/// 导出所有 inst_relate 实体（Prepack LOD 格式）
///
/// # 参数
/// - `owner_types`: 可选 owner_type 过滤（如 ["BRAN", "HANG"]），默认不过滤但仍排除 EQUI
/// - `name_config`: 可选名称配置，用于将三维模型节点名称转换为 PID 对象名称
/// - `export_all_lods`: 是否导出所有 LOD 级别，为 false 时仅导出 L1
pub async fn export_all_relates_prepack_lod(
    dbnum: Option<u32>,
    verbose: bool,
    output_override: Option<PathBuf>,
    owner_types: Option<Vec<String>>,
    name_config: Option<super::name_config::NameConfig>,
    db_option: Arc<DbOption>,
    export_all_lods: bool,
    export_refnos: Option<String>,
    source_unit: String,
    target_unit: String,
) -> Result<()> {
    use aios_core::rs_surreal::query_ext::SurrealQueryExt;
    use std::collections::HashSet;

    fn parse_length_unit(unit: &str) -> LengthUnit {
        match unit.to_lowercase().as_str() {
            "mm" => LengthUnit::Millimeter,
            "cm" => LengthUnit::Centimeter,
            "dm" => LengthUnit::Decimeter,
            "m" => LengthUnit::Meter,
            "in" => LengthUnit::Inch,
            "ft" => LengthUnit::Foot,
            "yd" => LengthUnit::Yard,
            _ => LengthUnit::Millimeter,
        }
    }

    // 解析单位参数
    let source_length_unit = parse_length_unit(&source_unit);
    let target_length_unit = parse_length_unit(&target_unit);

    println!("\n🔍 查询 inst_relate 表...");

    // 2.1 如果指定了 export_refnos，直接解析并导出
    if let Some(ref refnos_str) = export_refnos {
        let refnos: Vec<RefnoEnum> = refnos_str
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                if s.is_empty() {
                    return None;
                }
                Some(RefnoEnum::from(s))
            })
            .collect();

        if refnos.is_empty() {
            println!("   ⚠️ 未指定有效的 refno");
            return Ok(());
        }

        println!("   🎯 导出 {} 个指定 refnos", refnos.len());

        // 确定输出目录
        let output_dir = if let Some(custom) = output_override {
            custom
        } else {
            PathBuf::from(format!("output/export_{}", refnos_str.replace(',', "_")))
        };

        println!("\n🔄 导出 Prepack LOD 格式:");
        println!("   - 输出目录: {}", output_dir.display());
        println!("   - 总实体数: {}", refnos.len());

        // 获取 mesh 目录
        let mesh_dir = if let Some(ref path) = db_option.meshes_path {
            PathBuf::from(path)
        } else {
            PathBuf::from("assets/meshes")
        };

        return export_prepack_lod_for_refnos(
            &refnos,
            &mesh_dir,
            &output_dir,
            db_option,
            true,  // include_descendants - 改为 true 以包含子实例
            owner_types,  // filter_nouns
            verbose,
            name_config.as_ref(),
            export_all_lods,
            source_length_unit,
            target_length_unit,
        )
        .await;
    }

    // 2. 可选 owner_type 过滤
    let normalized_owner_types = owner_types
        .as_ref()
        .map(|types| types.iter().map(|t| t.to_uppercase()).collect::<Vec<_>>());

    // 先通过 Noun 获取核心入口，但只在未指定 owner_types 时使用
    let noun_roots = {
        if normalized_owner_types.is_some() {
            // 如果指定了 owner_types，不通过 Noun 查询，完全依赖 inst_relate 过滤
            Vec::new()
        } else {
            // 未指定 owner_types 时，才通过 Noun 获取 BRAN/EQUI 作为入口
            let nouns = ["BRAN", "EQUI"];
            if let Some(dbnum) = dbnum {
                query_provider::query_by_type(&nouns, dbnum as i32, None)
                    .await
                    .unwrap_or_default()
            } else {
                query_provider::query_by_noun_all_db(&nouns)
                    .await
                    .unwrap_or_default()
            }
        }
    };

    // 为 BRAN/HANG 准备名称映射（使用 full name）
    let mut refno_name_map: HashMap<RefnoEnum, String> = HashMap::new();
    if !noun_roots.is_empty() {
        for refno in &noun_roots {
            if let Ok(full_name) = aios_core::get_default_full_name(*refno).await {
                if !full_name.is_empty() {
                    // 只去掉开头的斜线，保持其他字符原样
                    let trimmed_name = full_name.trim().trim_start_matches('/').to_string();
                    if !trimmed_name.is_empty() {
                        // 如果有名称配置，尝试转换名称；否则保持原样
                        let final_name = if let Some(ref config) = name_config {
                            // convert_name 如果没有匹配会返回原名称
                            config.convert_name(&trimmed_name)
                        } else {
                            trimmed_name
                        };
                        refno_name_map.insert(*refno, final_name);
                    }
                }
            }
        }
    }

    // 1. 可选按 dbnum 限定 inst_relate 范围
    //    如果提供了 dbnum，只查询该 db 下的 inst_relate；否则全表扫描。
    let db_filter = if let Some(dbnum) = dbnum {
        println!("   - 模式: 按 dbnum={} 过滤", dbnum);
        format!("in.dbnum = {} ", dbnum)
    } else {
        println!("   - 模式: 全表扫描（所有 dbnum）");
        "1=1 ".to_string()
    };

    // P2 修复：移除重复的 normalized_owner_types 声明，复用上面第 1608 行的变量
    let owner_filter_clause =
        if let Some(types) = normalized_owner_types.as_ref().filter(|v| !v.is_empty()) {
            let list = types
                .iter()
                .map(|t| format!("'{}'", t))
                .collect::<Vec<_>>()
                .join(", ");
            println!("   - 按 owner_type 过滤 inst_relate: {:?}", types);
            // 修复：只按 owner_type 过滤，不使用 generic 字段避免不精确匹配
            format!(" AND owner_type IN [{list}]")
        } else {
            println!("   - 未指定 owner_type 过滤（仅排除 EQUI）");
            String::new()
        };

    // 3. 筛出 owner_type = 'EQUI' 的 inst_relate，用于设备分租信息（始终排除）
    let equi_sql = format!(
        "SELECT value in.id FROM inst_relate WHERE {} AND owner_type = 'EQUI'",
        db_filter
    );
    let equi_refnos: Vec<RefnoEnum> = aios_core::SUL_DB.query_take(&equi_sql, 0).await?;
    println!(
        "   - 找到 {} 条 owner_type = 'EQUI' 的 inst_relate 记录（用于设备分组）",
        equi_refnos.len()
    );
    let equi_set: HashSet<RefnoEnum> = equi_refnos.into_iter().collect();

    // 4. 再次扫描 inst_relate，收集需要导出的实体（不按 owner_type 过滤，仅排除 EQUI）
    let sql_all = format!(
        "SELECT value in.id FROM inst_relate WHERE {}{} AND record::exists(type::record('inst_relate_aabb', record::id(in)))",
        db_filter, owner_filter_clause
    );
    let mut all_refnos: Vec<RefnoEnum> = aios_core::SUL_DB.query_take(&sql_all, 0).await?;

    // 只有在未指定 owner_types 时才添加 noun_roots，避免绕过过滤
    if normalized_owner_types.is_none() {
        all_refnos.extend(noun_roots.into_iter());
    }

    let mut unique_refnos = HashSet::new();
    let mut refnos = Vec::new();
    for r in all_refnos {
        // 跳过 owner_type 为 EQUI 的 inst_relate（设备节点），只保留实际实体
        if equi_set.contains(&r) {
            continue;
        }
        if unique_refnos.insert(r.clone()) {
            refnos.push(r);
        }
    }

    println!(
        "      - 最终需要导出的 inst_relate 实体数: {} (已排除 EQUI 节点，含 Noun 入口)",
        refnos.len()
    );

    if refnos.is_empty() {
        println!("⚠️  未找到任何 inst_relate 实体");
        return Ok(());
    }

    // 确定输出目录
    let output_dir = if let Some(custom) = output_override {
        custom
    } else if let Some(dbnum) = dbnum {
        PathBuf::from(format!("output/all_relates_dbno_{}", dbnum))
    } else {
        PathBuf::from("output/all_relates_all")
    };

    println!("\n🔄 导出 Prepack LOD 格式:");
    println!("   - 输出目录: {}", output_dir.display());
    println!("   - 总实体数: {}", refnos.len());

    // 调用导出函数
    let mesh_dir = db_option.get_meshes_path();
    export_prepack_lod_for_refnos(
        &refnos,
        &mesh_dir,
        &output_dir,
        db_option,
        false, // include_descendants
        None,  // filter_nouns
        verbose,
        name_config.as_ref(),
        export_all_lods,
        source_length_unit,
        target_length_unit,
    )
    .await?;

    println!("\n🎉 Prepack LOD 导出完成！");
    Ok(())
}

pub async fn export_all_relates_prepack_lod_parquet(
    dbnum: Option<u32>,
    verbose: bool,
    output_override: Option<PathBuf>,
    owner_types: Option<Vec<String>>,
    name_config: Option<super::name_config::NameConfig>,
    db_option: Arc<DbOption>,
    export_all_lods: bool,
    export_refnos: Option<String>,
    source_unit: String,
    target_unit: String,
) -> Result<()> {
    use aios_core::rs_surreal::query_ext::SurrealQueryExt;
    use std::collections::HashSet;

    fn parse_length_unit(unit: &str) -> LengthUnit {
        match unit.to_lowercase().as_str() {
            "mm" => LengthUnit::Millimeter,
            "cm" => LengthUnit::Centimeter,
            "dm" => LengthUnit::Decimeter,
            "m" => LengthUnit::Meter,
            "in" => LengthUnit::Inch,
            "ft" => LengthUnit::Foot,
            "yd" => LengthUnit::Yard,
            _ => LengthUnit::Millimeter,
        }
    }

    let source_length_unit = parse_length_unit(&source_unit);
    let target_length_unit = parse_length_unit(&target_unit);

    println!("\n🔍 查询 inst_relate 表...");

    if let Some(ref refnos_str) = export_refnos {
        let refnos: Vec<RefnoEnum> = refnos_str
            .split(',')
            .filter_map(|s| {
                let s = s.trim();
                if s.is_empty() {
                    return None;
                }
                Some(RefnoEnum::from(s))
            })
            .collect();

        if refnos.is_empty() {
            println!("   ⚠️ 未指定有效的 refno");
            return Ok(());
        }

        println!("   🎯 导出 {} 个指定 refnos", refnos.len());

        let mut visible_refnos =
            collect_deep_visible_refnos(&refnos, verbose, owner_types.as_deref()).await?;
        if visible_refnos.is_empty() {
            println!("   ⚠️ 未找到深度可见实例，回退到输入 refnos");
            visible_refnos = refnos.clone();
        }
        println!("   - 深度可见节点数: {}", visible_refnos.len());

        let output_dir = if let Some(custom) = output_override {
            custom
        } else {
            PathBuf::from(format!("output/export_{}", refnos_str.replace(',', "_")))
        };

        println!("\n🔄 导出 Prepack LOD 格式:");
        println!("   - 输出目录: {}", output_dir.display());
        println!("   - 总实体数: {}", visible_refnos.len());

        let mesh_dir = if let Some(ref path) = db_option.meshes_path {
            PathBuf::from(path)
        } else {
            PathBuf::from("assets/meshes")
        };

        export_prepack_lod_for_refnos(
            &visible_refnos,
            &mesh_dir,
            &output_dir,
            db_option,
            false,
            None,
            verbose,
            name_config.as_ref(),
            export_all_lods,
            source_length_unit,
            target_length_unit,
        )
        .await?;

        write_prepack_parquet_and_patch_manifest(&output_dir)?;
        return Ok(());
    }

    let normalized_owner_types = owner_types
        .as_ref()
        .map(|types| types.iter().map(|t| t.to_uppercase()).collect::<Vec<_>>());

    let noun_roots = {
        if normalized_owner_types.is_some() {
            Vec::new()
        } else {
            let nouns = ["BRAN", "EQUI"];
            if let Some(dbnum) = dbnum {
                query_provider::query_by_type(&nouns, dbnum as i32, None)
                    .await
                    .unwrap_or_default()
            } else {
                query_provider::query_by_noun_all_db(&nouns)
                    .await
                    .unwrap_or_default()
            }
        }
    };

    let mut refno_name_map: HashMap<RefnoEnum, String> = HashMap::new();
    if !noun_roots.is_empty() {
        for refno in &noun_roots {
            if let Ok(full_name) = aios_core::get_default_full_name(*refno).await {
                if !full_name.is_empty() {
                    let trimmed_name = full_name.trim().trim_start_matches('/').to_string();
                    if !trimmed_name.is_empty() {
                        let final_name = if let Some(ref config) = name_config {
                            config.convert_name(&trimmed_name)
                        } else {
                            trimmed_name
                        };
                        refno_name_map.insert(*refno, final_name);
                    }
                }
            }
        }
    }

    let db_filter = if let Some(dbnum) = dbnum {
        println!("   - 模式: 按 dbnum={} 过滤", dbnum);
        format!("in.dbnum = {} ", dbnum)
    } else {
        println!("   - 模式: 全表扫描（所有 dbnum）");
        "1=1 ".to_string()
    };

    let normalized_owner_types = owner_types
        .as_ref()
        .map(|types| types.iter().map(|t| t.to_uppercase()).collect::<Vec<_>>());
    let owner_filter_clause =
        if let Some(types) = normalized_owner_types.as_ref().filter(|v| !v.is_empty()) {
            let list = types
                .iter()
                .map(|t| format!("'{}'", t))
                .collect::<Vec<_>>()
                .join(", ");
            println!("   - 按 owner_type 过滤 inst_relate: {:?}", types);
            format!(" AND owner_type IN [{list}]")
        } else {
            println!("   - 未指定 owner_type 过滤（仅排除 EQUI）");
            String::new()
        };

    let equi_sql = format!(
        "SELECT value in.id FROM inst_relate WHERE {} AND owner_type = 'EQUI'",
        db_filter
    );
    let equi_refnos: Vec<RefnoEnum> = aios_core::SUL_DB.query_take(&equi_sql, 0).await?;
    println!(
        "   - 找到 {} 条 owner_type = 'EQUI' 的 inst_relate 记录（用于设备分组）",
        equi_refnos.len()
    );
    let equi_set: HashSet<RefnoEnum> = equi_refnos.into_iter().collect();

    let sql_all = format!(
        "SELECT value in.id FROM inst_relate WHERE {}{} AND record::exists(type::record('inst_relate_aabb', record::id(in)))",
        db_filter, owner_filter_clause
    );
    let mut all_refnos: Vec<RefnoEnum> = aios_core::SUL_DB.query_take(&sql_all, 0).await?;

    if normalized_owner_types.is_none() {
        all_refnos.extend(noun_roots.into_iter());
    }

    let mut unique_refnos = HashSet::new();
    let mut refnos = Vec::new();
    for r in all_refnos {
        if equi_set.contains(&r) {
            continue;
        }
        if unique_refnos.insert(r.clone()) {
            refnos.push(r);
        }
    }

    println!(
        "      - 最终需要导出的 inst_relate 实体数: {} (已排除 EQUI 节点，含 Noun 入口)",
        refnos.len()
    );

    if refnos.is_empty() {
        println!("⚠️  未找到任何 inst_relate 实体");
        return Ok(());
    }

    let output_dir = if let Some(custom) = output_override {
        custom
    } else if let Some(dbnum) = dbnum {
        PathBuf::from(format!("output/all_relates_dbno_{}", dbnum))
    } else {
        PathBuf::from("output/all_relates_all")
    };

    println!("\n🔄 导出 Prepack LOD 格式:");
    println!("   - 输出目录: {}", output_dir.display());
    println!("   - 总实体数: {}", refnos.len());

    let mesh_dir = db_option.get_meshes_path();
    export_prepack_lod_for_refnos(
        &refnos,
        &mesh_dir,
        &output_dir,
        db_option,
        false,
        None,
        verbose,
        name_config.as_ref(),
        export_all_lods,
        source_length_unit,
        target_length_unit,
    )
    .await?;

    write_prepack_parquet_and_patch_manifest(&output_dir)?;

    println!("\n🎉 Prepack LOD 导出完成！");
    Ok(())
}

#[derive(Deserialize)]
struct PrepackInstancesV2 {
    version: Option<u32>,
    generated_at: String,
    colors: Option<Vec<[f32; 4]>>,
    bran_groups: Option<Vec<PrepackHierarchyGroup>>,
    equi_groups: Option<Vec<PrepackHierarchyGroup>>,
    ungrouped: Option<Vec<PrepackComponent>>,
}

#[derive(Deserialize)]
struct PrepackHierarchyGroup {
    refno: String,
    noun: Option<String>,
    name: Option<String>,
    children: Option<Vec<PrepackComponent>>,
    tubings: Option<Vec<PrepackTubing>>,
}

#[derive(Deserialize)]
struct PrepackComponent {
    refno: String,
    noun: String,
    name: Option<String>,
    color_index: usize,
    lod_mask: u32,
    spec_value: Option<i64>,
    refno_transform: Vec<f32>,
    instances: Vec<PrepackGeoInstance>,
}

#[derive(Deserialize)]
struct PrepackGeoInstance {
    geo_hash: String,
    geo_index: usize,
    geo_transform: Vec<f32>,
}

#[derive(Deserialize)]
struct PrepackTubing {
    refno: String,
    noun: Option<String>,
    name: Option<String>,
    geo_hash: String,
    geo_index: usize,
    matrix: Vec<f32>,
    color_index: usize,
    lod_mask: u32,
    spec_value: Option<i64>,
}

#[derive(Deserialize)]
struct PrepackGeometryManifest {
    generated_at: String,
    geometries: Vec<PrepackGeometryEntry>,
}

#[derive(Deserialize)]
struct PrepackGeometryEntry {
    geo_hash: String,
    geo_index: usize,
    nouns: Vec<String>,
    vertex_count: usize,
    triangle_count: usize,
    bounding_box: Option<PrepackBoundingBox>,
    bounding_sphere: Option<PrepackBoundingSphere>,
    lods: Vec<PrepackGeometryLod>,
}

#[derive(Deserialize)]
struct PrepackBoundingBox {
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Deserialize)]
struct PrepackBoundingSphere {
    center: [f32; 3],
    radius: f32,
}

#[derive(Deserialize)]
struct PrepackGeometryLod {
    level: u32,
    asset_key: String,
    mesh_index: usize,
    node_index: usize,
    triangle_count: usize,
    error_metric: f32,
}

// ============================================================================
// export_dbnum_instances_json 相关函数
// ============================================================================

/// 将 PlantAabb 转换为 JSON 格式 { "min": [x, y, z], "max": [x, y, z] }
/// 支持可选的单位转换
fn aabb_to_json(aabb: &aios_core::types::PlantAabb, unit_converter: &UnitConverter) -> serde_json::Value {
    let factor = if unit_converter.needs_conversion() {
        unit_converter.conversion_factor() as f32
    } else {
        1.0
    };
    json!({
        "min": [
            aabb.0.mins.x * factor,
            aabb.0.mins.y * factor,
            aabb.0.mins.z * factor
        ],
        "max": [
            aabb.0.maxs.x * factor,
            aabb.0.maxs.y * factor,
            aabb.0.maxs.z * factor
        ],
    })
}

/// 计算 BRAN/EQUI Owner 的 AABB（Union 所有 children 和 tubi 的 AABB）
fn compute_owner_aabb(
    children_aabbs: &[Option<aios_core::types::PlantAabb>],
    tubings_aabbs: &[Option<aios_core::types::PlantAabb>],
) -> Option<aios_core::types::PlantAabb> {
    use parry3d::bounding_volume::Aabb;

    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut min_z = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut max_z = f64::MIN;
    let mut has_valid_aabb = false;

    // 合并所有子组件的 AABB
    for aabb in children_aabbs.iter().flatten() {
        has_valid_aabb = true;
        min_x = min_x.min(aabb.0.mins.x as f64);
        min_y = min_y.min(aabb.0.mins.y as f64);
        min_z = min_z.min(aabb.0.mins.z as f64);
        max_x = max_x.max(aabb.0.maxs.x as f64);
        max_y = max_y.max(aabb.0.maxs.y as f64);
        max_z = max_z.max(aabb.0.maxs.z as f64);
    }

    // 合并所有 tubi 的 AABB
    for aabb in tubings_aabbs.iter().flatten() {
        has_valid_aabb = true;
        min_x = min_x.min(aabb.0.mins.x as f64);
        min_y = min_y.min(aabb.0.mins.y as f64);
        min_z = min_z.min(aabb.0.mins.z as f64);
        max_x = max_x.max(aabb.0.maxs.x as f64);
        max_y = max_y.max(aabb.0.maxs.y as f64);
        max_z = max_z.max(aabb.0.maxs.z as f64);
    }

    if !has_valid_aabb {
        None
    } else {
        let combined_aabb = Aabb::new(
            parry3d::math::Point::new(min_x as f32, min_y as f32, min_z as f32),
            parry3d::math::Point::new(max_x as f32, max_y as f32, max_z as f32),
        );
        Some(aios_core::types::PlantAabb(combined_aabb))
    }
}

/// tubi_relate 查询结果结构体
#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct TubiQueryResult {
    pub refno: RefnoEnum,
    pub index: Option<i64>,
    pub leave: RefnoEnum,
    pub world_aabb: Option<aios_core::types::PlantAabb>,
    pub world_trans: Option<aios_core::PlantTransform>,
    pub world_aabb_hash: Option<String>,   // V3: aabb hash
    pub world_trans_hash: Option<String>,  // V3: trans hash
    pub geo_hash: Option<String>,
    pub spec_value: Option<i64>,
}

fn plant_transform_to_dmat4(t: &aios_core::PlantTransform) -> DMat4 {
    t.0.to_matrix().as_dmat4()
}

/// 非聚合类型查询结果结构体
#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct UngroupedInstanceResult {
    pub refno: RefnoEnum,
    pub noun: Option<String>,
    pub name: Option<String>,
}

/// TreeIndex 驱动的 inst_relate 查询结果结构体
#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct InstRelateRow {
    pub owner_refno: Option<RefnoEnum>,
    pub owner_type: Option<String>,
    pub refno: RefnoEnum,
    pub noun: Option<String>,
    pub name: Option<String>,
    pub aabb_hash: Option<String>,
    pub spec_value: Option<i64>,
}

/// 使用 TreeIndex refno 列表分批查询 inst_relate（避免全表扫描）
async fn query_inst_relate_rows_by_refnos(
    refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<Vec<InstRelateRow>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    const BATCH_SIZE: usize = 500;
    let mut rows = Vec::new();

    for (idx, chunk) in refnos.chunks(BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 inst_relate_aabb 分批 {}/{} (批大小 {})",
                idx + 1,
                (refnos.len() + BATCH_SIZE - 1) / BATCH_SIZE,
                chunk.len()
            );
        }

        // 构建 inst_relate_aabb 的 ID 列表（使用尖括号格式，与保存时一致）
        let aabb_ids = chunk
            .iter()
            .map(|r| format!("inst_relate_aabb:⟨{}⟩", r.to_string()))
            .collect::<Vec<_>>()
            .join(", ");

        // 从 inst_relate_aabb 开始查询，反向关联到 inst_relate
        let sql = format!(
            r#"
            SELECT
                in<-inst_relate[0].owner_refno as owner_refno,
                in<-inst_relate[0].owner_type as owner_type,
                in as refno,
                in.noun as noun,
                fn::default_full_name(in) as name,
                record::id(out) as aabb_hash,
                in<-inst_relate[0].spec_value as spec_value
            FROM [{aabb_ids}]
            WHERE out.d != NONE
            "#
        );

        let mut chunk_rows: Vec<InstRelateRow> =
            aios_core::SUL_DB.query_take(&sql, 0).await?;
        rows.append(&mut chunk_rows);
    }

    Ok(rows)
}

/// 导出指定 dbnum 的实例数据为简化 JSON 格式（含 AABB）
///
/// # 参数
/// - `dbnum`: 数据库编号
/// - `output_dir`: 输出目录
/// - `db_option`: 数据库选项
/// - `verbose`: 是否输出详细日志
/// - `target_unit`: 目标单位（可选，默认为毫米）
///
/// # 返回
/// 导出统计信息
pub async fn export_dbnum_instances_json(
    dbnum: u32,
    output_dir: &Path,
    db_option: std::sync::Arc<DbOption>,
    verbose: bool,
    target_unit: Option<LengthUnit>,
) -> Result<ExportStats> {
    let start_time = std::time::Instant::now();

    // 目标单位（默认毫米）
    let target = target_unit.unwrap_or(LengthUnit::Millimeter);
    // 创建单位转换器（源单位：毫米，目标单位：用户指定或默认毫米）
    let unit_converter = UnitConverter::new(LengthUnit::Millimeter, target);
    // 关键前置条件说明：instances_*.json 的 geo_instances 依赖 inst_relate/geo_relate/inst_geo 落库数据。
    // 若仅生成树（gen_tree_only=true）或不落库（save_db=false）/不生成 mesh（gen_mesh=false），
    // 则导出时通常只能拿到树结构与 pe_transform，geo_instances 很可能为空。
    let save_db = db_option.save_db.unwrap_or(false);
    let gen_tree_only = db_option.gen_tree_only;
    let gen_mesh = db_option.gen_mesh;

    if !save_db || gen_tree_only || !gen_mesh {
        eprintln!(
            "⚠️  当前配置可能导致导出的 geo_instances 为空：save_db={:?}, gen_tree_only={}, gen_mesh={}",
            db_option.save_db, db_option.gen_tree_only, db_option.gen_mesh
        );
        eprintln!(
            "   建议：先用 save_db=true, gen_tree_only=false, gen_mesh=true 跑一次模型生成/落库，再执行 --export-dbnum-instances-json"
        );
    }

    if verbose {
        println!("🚀 开始导出 dbnum={} 的实例数据，目标单位: {:?}", dbnum, target);
    }

    // 确保输出目录存在
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;

    // 1. 使用 TreeIndex 获取 dbnum 下的所有 refno
    if verbose {
        println!("🔍 加载 TreeIndex...");
    }
    let tree_manager = TreeIndexManager::with_default_dir(vec![dbnum]);
    let tree_path = tree_manager.tree_dir().join(format!("{}.tree", dbnum));
    if !tree_path.is_file() {
        return Err(anyhow::anyhow!(
            "TreeIndex 文件不存在: {}，请先生成 scene_tree",
            tree_path.display()
        ));
    }
    let tree_index = tree_manager
        .load_index(dbnum)
        .with_context(|| format!("加载 TreeIndex 失败: {}", tree_path.display()))?;
    let mut all_refnos: Vec<RefnoEnum> = tree_index
        .all_refnos()
        .into_iter()
        .map(RefnoEnum::from)
        .collect();
    all_refnos.sort_by_key(|r| r.to_string());
    if verbose {
        println!("✅ TreeIndex 加载完成，refno 数量: {}", all_refnos.len());
    }

    // 2. 分批查询 inst_relate 记录（避免全表扫描）
    if verbose {
        println!("🔍 按 TreeIndex refno 查询 inst_relate...");
    }
    let inst_rows = query_inst_relate_rows_by_refnos(&all_refnos, verbose).await?;
    if verbose {
        println!("✅ inst_relate 命中记录: {}", inst_rows.len());
    }
    
    // 转换为内部数据结构
    #[derive(Clone, Debug)]
    struct ChildRow {
        refno: RefnoEnum,
        noun: Option<String>,
        name: Option<String>,
        aabb_hash: Option<String>,  // V3: aabb hash
        spec_value: Option<i64>,
    }

    #[derive(Clone, Debug)]
    struct OwnerGroup {
        owner_type: String,
        children: Vec<ChildRow>,
    }

    let mut owner_groups: HashMap<RefnoEnum, OwnerGroup> = HashMap::new();
    let mut in_refnos: Vec<RefnoEnum> = Vec::new();
    let mut in_refno_set: HashSet<RefnoEnum> = HashSet::new();
    let mut instance_rows: Vec<UngroupedInstanceResult> = Vec::new();

    for row in inst_rows {
        let owner_type = row
            .owner_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(owner_type.as_str(), "BRAN" | "HANG" | "EQUI") {
            let Some(owner_refno) = row.owner_refno else {
                if verbose {
                    println!(
                        "⚠️ inst_relate refno={} 缺少 owner_refno，已跳过",
                        row.refno
                    );
                }
                continue;
            };
            if in_refno_set.insert(row.refno) {
                in_refnos.push(row.refno);
            }
            let entry = owner_groups.entry(owner_refno).or_insert_with(|| OwnerGroup {
                owner_type: owner_type.clone(),
                children: Vec::new(),
            });
            if entry.owner_type.is_empty() {
                entry.owner_type = owner_type.clone();
            }
            entry.children.push(ChildRow {
                refno: row.refno,
                noun: row.noun,
                name: row.name,
                aabb_hash: row.aabb_hash,
                spec_value: row.spec_value,
            });
        } else {
            instance_rows.push(UngroupedInstanceResult {
                refno: row.refno,
                noun: row.noun,
                name: row.name,
            });
        }
    }

    // owner 输出顺序稳定（便于 diff / cache）
    let mut owner_refnos: Vec<RefnoEnum> = owner_groups.keys().copied().collect();
    owner_refnos.sort_by_key(|r| r.to_string());

    if verbose {
        println!("✅ 查询到 {} 个分组（BRAN/HANG/EQUI），共 {} 个子节点", 
            owner_refnos.len(), in_refnos.len());
        println!("✅ 非聚合类型实例数量: {}", instance_rows.len());
    }

    // 4. 查询 tubi_relate 数据（仅 BRAN/HANG owner）
    let tubi_owner_refnos: Vec<RefnoEnum> = owner_refnos
        .iter()
        .filter(|r| match owner_groups.get(r) {
            Some(g) => matches!(g.owner_type.as_str(), "BRAN" | "HANG"),
            None => false,
        })
        .copied()
        .collect();
    let mut tubings_map: HashMap<RefnoEnum, Vec<TubiRecord>> = HashMap::new();

    if !tubi_owner_refnos.is_empty() {
        // 将多个 owner 的 ranges 查询打包成多语句一次执行（分批，避免 SQL 过长）
        for owners_chunk in tubi_owner_refnos.chunks(50) {
            let mut sql_batch = String::new();
            for owner_refno in owners_chunk {
                let pe_key = owner_refno.to_pe_key();
                sql_batch.push_str(&format!(
                    r#"
                    SELECT
                        id[0] as refno,
                        id[1] as index,
                        in as leave,
                        id[0].owner.noun as generic,
                        aabb.d as world_aabb,
                        world_trans.d as world_trans,
                        record::id(aabb) as world_aabb_hash,
                        record::id(world_trans) as world_trans_hash,
                        record::id(geo) as geo_hash,
                        id[0].dt as date,
                        spec_value
                    FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                    "#,
                ));
            }

            let mut resp = aios_core::SUL_DB.query_response(&sql_batch).await?;
            for (stmt_idx, owner_refno) in owners_chunk.iter().enumerate() {
                let raw_tubi_rows: Vec<TubiQueryResult> = resp.take(stmt_idx)?;
                for raw_tubi_row in raw_tubi_rows {
                    let Some(geo_hash) = raw_tubi_row.geo_hash else {
                        continue;
                    };

                    let index = raw_tubi_row
                        .index
                        .and_then(|v| usize::try_from(v).ok())
                        .unwrap_or(0);

                    let transform = raw_tubi_row
                        .world_trans
                        .as_ref()
                        .map(plant_transform_to_dmat4)
                        .unwrap_or(DMat4::IDENTITY);

                    let tubi_record = TubiRecord {
                        refno: raw_tubi_row.refno,
                        owner_refno: raw_tubi_row.leave,
                        geo_hash,
                        transform,
                        index,
                        name: format!("TUBI-{}-{}", raw_tubi_row.refno.to_string(), index),
                        spec_value: raw_tubi_row.spec_value,
                        aabb: raw_tubi_row.world_aabb,
                        world_aabb_hash: raw_tubi_row.world_aabb_hash,
                        world_trans_hash: raw_tubi_row.world_trans_hash,
                    };

                    tubings_map.entry(*owner_refno).or_default().push(tubi_record);
                }
            }
        }

        // 确保每个 owner 的 tubi 按 index 顺序输出（order 字段稳定）
        for tubis in tubings_map.values_mut() {
            tubis.sort_by_key(|t| t.index);
        }
    }

    if verbose {
        let total_tubis: usize = tubings_map.values().map(|v| v.len()).sum();
        println!("✅ 查询到 {} 条 tubi_relate 记录", total_tubis);
    }

    // 4. 批量查询所有 children 的几何体实例数据（只查询 hash 引用，不查询实际数据）
    // 使用 query_insts_for_export 直接获取数据库中的 hash ID
    let mut export_inst_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();
    if !in_refnos.is_empty() {
        if verbose {
            println!("🔍 查询 {} 个 refno 的几何体实例 hash...", in_refnos.len());
        }
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
                if verbose {
                    println!("⚠️ 几何体实例查询失败: {:?}", e);
                }
            }
        }
    }

    // 5. 构建简化的 JSON 结构
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let mut groups = Vec::new();

    for owner_refno in &owner_refnos {
        let Some(owner_group) = owner_groups.get(owner_refno) else {
            continue;
        };

        let owner_type = owner_group.owner_type.as_str();
        let owner_name = format!("{}-{}", owner_type, owner_refno.to_string());

        // 构建 children 数组 (V3: 直接使用数据库 hash 引用)
        let mut children = Vec::new();
        for child in &owner_group.children {
            let child_refno = &child.refno;

            // 从 export_inst_map 获取几何体实例的 hash 引用
            let export_inst = export_inst_map.get(child_refno);

            // 过滤：只导出有几何体的实例
            let Some(export_inst) = export_inst else {
                continue;  // 没有几何体实例，跳过
            };

            if export_inst.insts.is_empty() {
                continue;  // geo_instances 为空，跳过
            }

            // 由于 SQL 已过滤，这里的 aabb_hash 应该总是存在
            // 但为了安全起见，仍然检查
            let child_aabb_hash = export_inst.world_aabb_hash.clone()
                .or_else(|| child.aabb_hash.clone());
            if child_aabb_hash.is_none() {
                continue;  // 双重保险：确保有 AABB
            }

            let child_name = child
                .name
                .as_deref()
                .unwrap_or_default()
                .trim()
                .trim_start_matches('/')
                .to_string();
            let spec_value = child.spec_value;

            // 使用数据库中的 trans_hash
            let refno_trans_hash_str = export_inst.world_trans_hash.clone().unwrap_or_default();

            // 构建 geo_instances（直接使用数据库 hash）
            let instances: Vec<serde_json::Value> = export_inst.insts
                .iter()
                .map(|inst| {
                    json!({
                        "geo_hash": inst.geo_hash,
                        "geo_trans_hash": inst.trans_hash.clone().unwrap_or_default(),
                    })
                })
                .collect();

            let noun = child.noun.as_deref().unwrap_or("");

            // 获取布尔运算标识
            let has_neg = export_inst.has_neg;

            children.push(json!({
                "refno": child_refno.to_string(),
                "noun": noun,
                "name": child_name,
                "aabb_hash": child_aabb_hash,
                "lod_mask": 1u32,
                "spec_value": spec_value.unwrap_or(0),
                "trans_hash": refno_trans_hash_str,
                "has_neg": has_neg,
                "geo_instances": instances,
            }));
        }

        // 构建 tubings 数组 (V3: 直接使用数据库 hash 引用)
        let mut tubings = Vec::new();
        if let Some(tubi_records) = tubings_map.get(owner_refno) {
            for tubi in tubi_records {
                // 过滤：只导出有 AABB 和 geo_hash 的 tubi
                if tubi.world_aabb_hash.is_none() || tubi.geo_hash.is_empty() {
                    continue;
                }

                tubings.push(json!({
                    "refno": tubi.refno.to_string(),
                    "noun": "TUBI",
                    "name": tubi.name,
                    "aabb_hash": tubi.world_aabb_hash,
                    "geo_hash": tubi.geo_hash,
                    "trans_hash": tubi.world_trans_hash.clone().unwrap_or_default(),
                    "order": tubi.index,
                    "lod_mask": 1u32,
                    "spec_value": tubi.spec_value.unwrap_or(0),
                }));
            }
        }

        // V3: owner 不再计算 union aabb，仅使用 children/tubings 的 hash
        groups.push(json!({
            "owner_refno": owner_refno.to_string(),
            "owner_noun": owner_type,
            "owner_name": owner_name,
            "children": children,
            "tubings": tubings,
        }));
    }

    // 6. 查询这些 refno 的几何实例 hash（只查询 hash 引用，不查询实际数据）
    let instance_refnos: Vec<RefnoEnum> = instance_rows.iter().map(|r| r.refno).collect();
    let mut instance_export_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();

    if !instance_refnos.is_empty() {
        // 查询几何实例 hash
        if verbose {
            println!("🔍 查询 {} 个实例的几何 hash...", instance_refnos.len());
        }
        match aios_core::query_insts_for_export(&instance_refnos, true).await {
            Ok(export_insts) => {
                for inst in export_insts {
                    instance_export_map.insert(inst.refno, inst);
                }
                if verbose {
                    println!("✅ 查询到 {} 个实例有几何体数据", instance_export_map.len());
                }
            }
            Err(e) => {
                if verbose {
                    println!("⚠️ 几何体实例查询失败: {:?}", e);
                }
            }
        }
    }

    // 8. 构建 instances 数组 (V3: 直接使用数据库 hash 引用)
    let mut instances = Vec::new();
    let total_instance_rows = instance_rows.len();  // 保存长度用于统计
    for row in instance_rows {
        // 从 instance_export_map 获取几何体实例的 hash 引用
        let export_inst = instance_export_map.get(&row.refno);

        // 过滤：只导出有几何体的实例
        let Some(export_inst) = export_inst else {
            continue;
        };

        if export_inst.insts.is_empty() || export_inst.world_aabb_hash.is_none() {
            continue;
        }

        // 使用数据库中的 trans_hash
        let refno_trans_hash = export_inst.world_trans_hash.clone().unwrap_or_default();

        // 构建 geo_instances（直接使用数据库 hash）
        let geo_instances: Vec<serde_json::Value> = export_inst.insts
            .iter()
            .map(|inst| {
                json!({
                    "geo_hash": inst.geo_hash,
                    "geo_trans_hash": inst.trans_hash.clone().unwrap_or_default(),
                })
            })
            .collect();

        // 使用数据库中的 aabb_hash
        let inst_aabb_hash = export_inst.world_aabb_hash.clone();

        // 获取布尔运算标识
        let has_neg = export_inst.has_neg;

        instances.push(json!({
            "refno": row.refno.to_string(),
            "noun": row.noun.unwrap_or_default(),
            "name": row.name.unwrap_or_default(),
            "aabb_hash": inst_aabb_hash,
            "trans_hash": refno_trans_hash,
            "has_neg": has_neg,
            "geo_instances": geo_instances,
        }));
    }

    // 计算过滤统计
    let total_children: usize = owner_groups.values().map(|g| g.children.len()).sum();
    let exported_children: usize = groups.iter()
        .filter_map(|g| g.get("children").and_then(|v| v.as_array()))
        .map(|a| a.len())
        .sum();

    let total_tubings: usize = tubings_map.values().map(|v| v.len()).sum();
    let exported_tubings: usize = groups.iter()
        .filter_map(|g| g.get("tubings").and_then(|v| v.as_array()))
        .map(|a| a.len())
        .sum();

    let filtered_children = total_children.saturating_sub(exported_children);
    let filtered_tubings = total_tubings.saturating_sub(exported_tubings);
    let filtered_instances = total_instance_rows.saturating_sub(instances.len());

    if verbose {
        println!("\n📊 导出统计:");
        println!("   - Groups: {}", groups.len());
        println!("   - Children: {} 导出 / {} 总数 (过滤 {})",
            exported_children, total_children, filtered_children);
        println!("   - Tubings: {} 导出 / {} 总数 (过滤 {})",
            exported_tubings, total_tubings, filtered_tubings);
        println!("   - Instances: {} 导出 / {} 总数 (过滤 {})",
            instances.len(), total_instance_rows, filtered_instances);

        let total_filtered = filtered_children + filtered_tubings + filtered_instances;
        if total_filtered > 0 {
            println!("   - 总计过滤: {} 个无几何体的记录", total_filtered);
        }
    }

    // 主 JSON（V3 格式，trans/aabb 通过全局文件加载）
    let instances_json = json!({
        "version": 3,
        "generated_at": generated_at,
        "groups": groups,
        "instances": instances,
    });

    // 写入主文件
    let output_path = output_dir.join(format!("instances_{}.json", dbnum));
    let json_str = serde_json::to_string_pretty(&instances_json)?;
    fs::write(&output_path, json_str)?;

    if verbose {
        println!("✅ 主 JSON 文件已写入: {}", output_path.display());
    }

    // 返回统计信息
    let stats = ExportStats {
        refno_count: owner_refnos.len(),
        descendant_count: in_refnos.len(),
        geometry_count: 0,
        mesh_files_found: 0,
        mesh_files_missing: 0,
        output_file_size: {
            fs::metadata(&output_path).map(|m| m.len()).unwrap_or(0)
        },
        elapsed_time: start_time.elapsed(),
        node_count: 0,
        mesh_count: 0,
    };

    Ok(stats)
}

/// 导出全局 trans.json 和 aabb.json（扫描整表）
pub async fn export_global_trans_aabb_json(
    output_dir: &Path,
    target_unit: Option<LengthUnit>,
    verbose: bool,
) -> Result<(usize, usize)> {
    use aios_core::SurrealQueryExt;

    let target = target_unit.unwrap_or(LengthUnit::Millimeter);
    let unit_converter = UnitConverter::new(LengthUnit::Millimeter, target);
    const PAGE_SIZE: usize = 5000;

    // 1. 导出 trans 表
    // trans 表存储的是 bevy Transform (translation, rotation, scale)，需要转换为 Mat4 列主序数组
    if verbose {
        println!("🔍 扫描 trans 表导出 trans 数据...");
    }
    
    #[derive(Debug, Deserialize, SurrealValue)]
    struct TransRow {
        hash: String,
        d: serde_json::Value,
    }
    
    let mut trans_table: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    let mut trans_offset = 0usize;
    loop {
        let trans_sql = format!(
            "SELECT record::id(id) as hash, d FROM trans LIMIT {} START {}",
            PAGE_SIZE, trans_offset
        );
        let trans_results: Vec<TransRow> = aios_core::SUL_DB
            .query_take(&trans_sql, 0)
            .await
            .unwrap_or_default();
        if trans_results.is_empty() {
            break;
        }
        if verbose {
            println!(
                "   - trans 分页: offset={} 本批={}",
                trans_offset,
                trans_results.len()
            );
        }
        for row in trans_results {
            // d 是 bevy Transform: { translation: [x,y,z], rotation: [x,y,z,w], scale: [x,y,z] }
            if let Some(obj) = row.d.as_object() {
                let translation = obj.get("translation")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        glam::DVec3::new(x, y, z)
                    })
                    .unwrap_or(glam::DVec3::ZERO);
                
                let rotation = obj.get("rotation")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let w = arr.get(3).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        glam::DQuat::from_xyzw(x, y, z, w)
                    })
                    .unwrap_or(glam::DQuat::IDENTITY);
                
                let scale = obj.get("scale")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        let x = arr.get(0).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let y = arr.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        let z = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0);
                        glam::DVec3::new(x, y, z)
                    })
                    .unwrap_or(glam::DVec3::ONE);
                
                // 构建 DMat4 并转换为列主序数组
                let mat = glam::DMat4::from_scale_rotation_translation(scale, rotation, translation);
                let arr = mat4_to_vec_dmat4(&mat, &unit_converter, false);
                trans_table.insert(row.hash, serde_json::json!(arr));
            }
        }
        trans_offset += PAGE_SIZE;
    }
    
    if verbose {
        println!("   成功解析 {} 条 trans 数据", trans_table.len());
    }
    
    // 2. 导出 aabb 表
    if verbose {
        println!("🔍 扫描 aabb 表导出 aabb 数据...");
    }
    
    #[derive(Debug, Deserialize, SurrealValue)]
    struct AabbRow {
        hash: String,
        d: Option<aios_core::types::PlantAabb>,
    }
    
    let mut aabb_table: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    let mut aabb_offset = 0usize;
    loop {
        let aabb_sql = format!(
            "SELECT record::id(id) as hash, d FROM aabb LIMIT {} START {}",
            PAGE_SIZE, aabb_offset
        );
        let aabb_results: Vec<AabbRow> = aios_core::SUL_DB
            .query_take(&aabb_sql, 0)
            .await
            .unwrap_or_default();
        if aabb_results.is_empty() {
            break;
        }
        if verbose {
            println!(
                "   - aabb 分页: offset={} 本批={}",
                aabb_offset,
                aabb_results.len()
            );
        }
        for row in aabb_results {
            if let Some(aabb) = row.d {
                aabb_table.insert(row.hash, aabb_to_json(&aabb, &unit_converter));
            }
        }
        aabb_offset += PAGE_SIZE;
    }
    
    // 3. 写入文件
    let trans_path = output_dir.join("trans.json");
    let aabb_path = output_dir.join("aabb.json");
    
    let trans_json: serde_json::Value = trans_table.clone().into_iter().collect();
    let aabb_json: serde_json::Value = aabb_table.clone().into_iter().collect();
    
    fs::write(&trans_path, serde_json::to_string(&trans_json)?)?;
    fs::write(&aabb_path, serde_json::to_string(&aabb_json)?)?;
    
    if verbose {
        println!("✅ trans.json 已写入: {} ({} 条)", trans_path.display(), trans_table.len());
        println!("✅ aabb.json 已写入: {} ({} 条)", aabb_path.display(), aabb_table.len());
    }
    
    Ok((trans_table.len(), aabb_table.len()))
}

/// 将 DMat4 转换为 f32 数组（列主序）
/// P1 修复：添加单位转换逻辑，与 mat4_to_vec 保持一致
fn mat4_to_vec_dmat4(mat: &DMat4, unit_converter: &UnitConverter, is_unit_mesh: bool) -> Vec<f32> {
    let mut cols = mat.to_cols_array();
    if unit_converter.needs_conversion() {
        let factor = unit_converter.conversion_factor() as f64;
        // Unit mesh：缩放旋转/缩放部分；普通 mesh：不缩放旋转/缩放部分（已在顶点上）
        if is_unit_mesh {
            // 缩放旋转部分（前3列）
            for i in 0..3 {
                cols[i] *= factor; // 第一列
                cols[4 + i] *= factor; // 第二列
                cols[8 + i] *= factor; // 第三列
            }
        }
        // 平移部分始终需要缩放（世界坐标必须单位转换）
        cols[12] *= factor;
        cols[13] *= factor;
        cols[14] *= factor;
    }
    cols.iter().map(|&v| v as f32).collect()
}

fn mat4_mul_col_major(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for i in 0..4 {
        for j in 0..4 {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k * 4 + i] * b[j * 4 + k];
            }
            out[j * 4 + i] = sum;
        }
    }
    out
}

fn as_mat4_16(v: &[f32]) -> Result<[f32; 16]> {
    let slice: [f32; 16] = v
        .get(0..16)
        .ok_or_else(|| anyhow::anyhow!("矩阵长度不足: {}", v.len()))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("矩阵转换失败"))?;
    Ok(slice)
}

fn write_prepack_parquet_and_patch_manifest(output_dir: &Path) -> Result<()> {
    let instances_path = output_dir.join("instances.json");
    let geometry_manifest_path = output_dir.join("geometry_manifest.json");
    let manifest_path = output_dir.join("manifest.json");

    let instances: PrepackInstancesV2 = serde_json::from_slice(&fs::read(&instances_path)?)?;
    let geom_manifest: PrepackGeometryManifest = serde_json::from_slice(&fs::read(&geometry_manifest_path)?)?;

    let instances_parquet_name = "instances.parquet";
    let geometry_manifest_parquet_name = "geometry_manifest.parquet";

    write_instances_parquet(output_dir.join(instances_parquet_name), &instances)?;
    write_geometry_manifest_parquet(output_dir.join(geometry_manifest_parquet_name), &geom_manifest)?;

    let mut manifest_json: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let Some(files_obj) = manifest_json.get_mut("files") else {
        return Ok(());
    };
    let Some(files_obj) = files_obj.as_object_mut() else {
        return Ok(());
    };

    let instances_file = output_dir.join(instances_parquet_name);
    let instances_meta = fs::metadata(&instances_file)?;
    let instances_sha = sha256_for_file(&instances_file)?;
    files_obj.insert(
        "instances_parquet".to_string(),
        json!({
            "path": instances_parquet_name,
            "bytes": instances_meta.len(),
            "sha256": instances_sha,
        }),
    );

    let geom_file = output_dir.join(geometry_manifest_parquet_name);
    let geom_meta = fs::metadata(&geom_file)?;
    let geom_sha = sha256_for_file(&geom_file)?;
    files_obj.insert(
        "geometry_manifest_parquet".to_string(),
        json!({
            "path": geometry_manifest_parquet_name,
            "bytes": geom_meta.len(),
            "sha256": geom_sha,
        }),
    );

    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest_json)?)?;
    Ok(())
}

fn write_instances_parquet(path: PathBuf, instances: &PrepackInstancesV2) -> Result<()> {
    let colors = instances.colors.clone().unwrap_or_default();

    let mut refnos: Vec<Option<String>> = Vec::new();
    let mut owner_nouns: Vec<Option<String>> = Vec::new();
    let mut owner_refnos: Vec<Option<String>> = Vec::new();
    let mut nouns: Vec<Option<String>> = Vec::new();
    let mut names: Vec<Option<String>> = Vec::new();
    let mut spec_values: Vec<Option<u32>> = Vec::new();
    let mut geo_hashes: Vec<Option<String>> = Vec::new();
    let mut geo_indices: Vec<u32> = Vec::new();
    let mut lod_masks: Vec<u32> = Vec::new();

    let mut color_builder = FixedSizeListBuilder::new(PrimitiveBuilder::<Float32Type>::new(), 4)
        .with_field(Arc::new(Field::new_list_field(DataType::Float32, false)));
    let mut mat_builder = FixedSizeListBuilder::new(PrimitiveBuilder::<Float32Type>::new(), 16)
        .with_field(Arc::new(Field::new_list_field(DataType::Float32, false)));

    let mut push_row = |entity_refno: String,
                        owner_noun: Option<String>,
                        owner_refno: Option<String>,
                        noun: Option<String>,
                        name: Option<String>,
                        spec_value: Option<i64>,
                        geo_hash: String,
                        geo_index: u32,
                        lod_mask: u32,
                        matrix: [f32; 16],
                        color: [f32; 4]| {
        refnos.push(Some(entity_refno));
        owner_nouns.push(owner_noun);
        owner_refnos.push(owner_refno);
        nouns.push(noun);
        names.push(name);
        spec_values.push(spec_value.and_then(|v| u32::try_from(v).ok()));
        geo_hashes.push(Some(geo_hash));
        geo_indices.push(geo_index);
        lod_masks.push(lod_mask);

        for v in color {
            color_builder.values().append_value(v);
        }
        color_builder.append(true);

        for v in matrix {
            mat_builder.values().append_value(v);
        }
        mat_builder.append(true);
    };

    if let Some(groups) = instances.bran_groups.as_ref() {
        for group in groups {
            let owner_refno = group.refno.clone();
            if let Some(children) = group.children.as_ref() {
                for component in children {
                    let refno_mat = as_mat4_16(&component.refno_transform)?;
                    for inst in &component.instances {
                        let geo_mat = as_mat4_16(&inst.geo_transform)?;
                        let matrix = mat4_mul_col_major(&refno_mat, &geo_mat);
                        let color = *colors
                            .get(component.color_index)
                            .unwrap_or(&[1.0, 1.0, 1.0, 1.0]);
                        push_row(
                            component.refno.clone(),
                            Some("BRAN".to_string()),
                            Some(owner_refno.clone()),
                            Some(component.noun.clone()),
                            component.name.clone(),
                            component.spec_value,
                            inst.geo_hash.clone(),
                            inst.geo_index as u32,
                            component.lod_mask,
                            matrix,
                            color,
                        );
                    }
                }
            }

            if let Some(tubings) = group.tubings.as_ref() {
                for tubing in tubings {
                    let matrix = as_mat4_16(&tubing.matrix)?;
                    let color = *colors
                        .get(tubing.color_index)
                        .unwrap_or(&[1.0, 1.0, 1.0, 1.0]);
                    push_row(
                        tubing.refno.clone(),
                        Some("BRAN".to_string()),
                        Some(owner_refno.clone()),
                        Some("TUBI".to_string()),
                        tubing.name.clone(),
                        tubing.spec_value,
                        tubing.geo_hash.clone(),
                        tubing.geo_index as u32,
                        tubing.lod_mask,
                        matrix,
                        color,
                    );
                }
            }
        }
    }

    if let Some(groups) = instances.equi_groups.as_ref() {
        for group in groups {
            let owner_refno = group.refno.clone();
            if let Some(children) = group.children.as_ref() {
                for component in children {
                    let refno_mat = as_mat4_16(&component.refno_transform)?;
                    for inst in &component.instances {
                        let geo_mat = as_mat4_16(&inst.geo_transform)?;
                        let matrix = mat4_mul_col_major(&refno_mat, &geo_mat);
                        let color = *colors
                            .get(component.color_index)
                            .unwrap_or(&[1.0, 1.0, 1.0, 1.0]);
                        push_row(
                            component.refno.clone(),
                            Some("EQUI".to_string()),
                            Some(owner_refno.clone()),
                            Some(component.noun.clone()),
                            component.name.clone(),
                            component.spec_value,
                            inst.geo_hash.clone(),
                            inst.geo_index as u32,
                            component.lod_mask,
                            matrix,
                            color,
                        );
                    }
                }
            }
        }
    }

    if let Some(ungrouped) = instances.ungrouped.as_ref() {
        for component in ungrouped {
            let refno_mat = as_mat4_16(&component.refno_transform)?;
            for inst in &component.instances {
                let geo_mat = as_mat4_16(&inst.geo_transform)?;
                let matrix = mat4_mul_col_major(&refno_mat, &geo_mat);
                let color = *colors
                    .get(component.color_index)
                    .unwrap_or(&[1.0, 1.0, 1.0, 1.0]);
                push_row(
                    component.refno.clone(),
                    None,
                    None,
                    Some(component.noun.clone()),
                    component.name.clone(),
                    component.spec_value,
                    inst.geo_hash.clone(),
                    inst.geo_index as u32,
                    component.lod_mask,
                    matrix,
                    color,
                );
            }
        }
    }

    let refno_refs: Vec<Option<&str>> = refnos.iter().map(|s| s.as_deref()).collect();
    let owner_noun_refs: Vec<Option<&str>> = owner_nouns.iter().map(|s| s.as_deref()).collect();
    let owner_refno_refs: Vec<Option<&str>> = owner_refnos.iter().map(|s| s.as_deref()).collect();
    let noun_refs: Vec<Option<&str>> = nouns.iter().map(|s| s.as_deref()).collect();
    let name_refs: Vec<Option<&str>> = names.iter().map(|s| s.as_deref()).collect();
    let geo_hash_refs: Vec<Option<&str>> = geo_hashes.iter().map(|s| s.as_deref()).collect();

    let refno_arr: ArrayRef = Arc::new(StringArray::from(refno_refs));
    let owner_noun_arr: ArrayRef = Arc::new(StringArray::from(owner_noun_refs));
    let owner_refno_arr: ArrayRef = Arc::new(StringArray::from(owner_refno_refs));
    let noun_arr: ArrayRef = Arc::new(StringArray::from(noun_refs));
    let name_arr: ArrayRef = Arc::new(StringArray::from(name_refs));
    let spec_arr: ArrayRef = Arc::new(UInt32Array::from(spec_values));
    let geo_hash_arr: ArrayRef = Arc::new(StringArray::from(geo_hash_refs));
    let geo_index_arr: ArrayRef = Arc::new(UInt32Array::from(geo_indices));
    let lod_mask_arr: ArrayRef = Arc::new(UInt32Array::from(lod_masks));
    let color_arr: ArrayRef = Arc::new(color_builder.finish());
    let mat_arr: ArrayRef = Arc::new(mat_builder.finish());

    let schema = Arc::new(Schema::new(vec![
        Field::new("refno", DataType::Utf8, true),
        Field::new("owner_noun", DataType::Utf8, true),
        Field::new("owner_refno", DataType::Utf8, true),
        Field::new("noun", DataType::Utf8, true),
        Field::new("name", DataType::Utf8, true),
        Field::new("spec_value", DataType::UInt32, true),
        Field::new("geo_hash", DataType::Utf8, true),
        Field::new("geo_index", DataType::UInt32, false),
        Field::new("lod_mask", DataType::UInt32, false),
        Field::new(
            "color_rgba",
            DataType::FixedSizeList(Arc::new(Field::new_list_field(DataType::Float32, false)), 4),
            false,
        ),
        Field::new(
            "matrix",
            DataType::FixedSizeList(Arc::new(Field::new_list_field(DataType::Float32, false)), 16),
            false,
        ),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            refno_arr,
            owner_noun_arr,
            owner_refno_arr,
            noun_arr,
            name_arr,
            spec_arr,
            geo_hash_arr,
            geo_index_arr,
            lod_mask_arr,
            color_arr,
            mat_arr,
        ],
    )?;

    let mut file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(&mut file, batch.schema(), None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

fn write_geometry_manifest_parquet(path: PathBuf, manifest: &PrepackGeometryManifest) -> Result<()> {
    let mut geo_hashes: Vec<Option<String>> = Vec::new();
    let mut geo_indices: Vec<u32> = Vec::new();
    let mut nouns_joined: Vec<Option<String>> = Vec::new();
    let mut vertex_counts: Vec<u32> = Vec::new();
    let mut tri_counts: Vec<u32> = Vec::new();
    let mut lod_levels: Vec<u32> = Vec::new();
    let mut asset_keys: Vec<Option<String>> = Vec::new();
    let mut mesh_indices: Vec<u32> = Vec::new();
    let mut node_indices: Vec<u32> = Vec::new();
    let mut lod_tri_counts: Vec<u32> = Vec::new();
    let mut error_metrics: Vec<f32> = Vec::new();
    let mut sphere_radii: Vec<Option<f32>> = Vec::new();

    let mut bbox_min_builder = FixedSizeListBuilder::new(PrimitiveBuilder::<Float32Type>::new(), 3)
        .with_field(Arc::new(Field::new_list_field(DataType::Float32, false)));
    let mut bbox_max_builder = FixedSizeListBuilder::new(PrimitiveBuilder::<Float32Type>::new(), 3)
        .with_field(Arc::new(Field::new_list_field(DataType::Float32, false)));
    let mut sphere_center_builder = FixedSizeListBuilder::new(PrimitiveBuilder::<Float32Type>::new(), 3)
        .with_field(Arc::new(Field::new_list_field(DataType::Float32, false)));

    for geo in &manifest.geometries {
        let joined = if geo.nouns.is_empty() {
            None
        } else {
            Some(geo.nouns.join(";"))
        };

        for lod in &geo.lods {
            geo_hashes.push(Some(geo.geo_hash.clone()));
            geo_indices.push(geo.geo_index as u32);
            nouns_joined.push(joined.clone());
            vertex_counts.push(geo.vertex_count as u32);
            tri_counts.push(geo.triangle_count as u32);
            lod_levels.push(lod.level);
            asset_keys.push(Some(lod.asset_key.clone()));
            mesh_indices.push(lod.mesh_index as u32);
            node_indices.push(lod.node_index as u32);
            lod_tri_counts.push(lod.triangle_count as u32);
            error_metrics.push(lod.error_metric);

            if let Some(b) = &geo.bounding_box {
                for v in b.min {
                    bbox_min_builder.values().append_value(v);
                }
                bbox_min_builder.append(true);

                for v in b.max {
                    bbox_max_builder.values().append_value(v);
                }
                bbox_max_builder.append(true);
            } else {
                bbox_min_builder.values().append_value(0.0);
                bbox_min_builder.values().append_value(0.0);
                bbox_min_builder.values().append_value(0.0);
                bbox_min_builder.append(true);
                bbox_max_builder.values().append_value(0.0);
                bbox_max_builder.values().append_value(0.0);
                bbox_max_builder.values().append_value(0.0);
                bbox_max_builder.append(true);
            }

            if let Some(s) = &geo.bounding_sphere {
                for v in s.center {
                    sphere_center_builder.values().append_value(v);
                }
                sphere_center_builder.append(true);
                sphere_radii.push(Some(s.radius));
            } else {
                sphere_center_builder.values().append_value(0.0);
                sphere_center_builder.values().append_value(0.0);
                sphere_center_builder.values().append_value(0.0);
                sphere_center_builder.append(true);
                sphere_radii.push(None);
            }
        }
    }

    let geo_hash_refs: Vec<Option<&str>> = geo_hashes.iter().map(|s| s.as_deref()).collect();
    let nouns_refs: Vec<Option<&str>> = nouns_joined.iter().map(|s| s.as_deref()).collect();
    let asset_refs: Vec<Option<&str>> = asset_keys.iter().map(|s| s.as_deref()).collect();

    let geo_hash_arr: ArrayRef = Arc::new(StringArray::from(geo_hash_refs));
    let geo_index_arr: ArrayRef = Arc::new(UInt32Array::from(geo_indices));
    let nouns_arr: ArrayRef = Arc::new(StringArray::from(nouns_refs));
    let vertex_arr: ArrayRef = Arc::new(UInt32Array::from(vertex_counts));
    let tri_arr: ArrayRef = Arc::new(UInt32Array::from(tri_counts));
    let lod_level_arr: ArrayRef = Arc::new(UInt32Array::from(lod_levels));
    let asset_arr: ArrayRef = Arc::new(StringArray::from(asset_refs));
    let mesh_arr: ArrayRef = Arc::new(UInt32Array::from(mesh_indices));
    let node_arr: ArrayRef = Arc::new(UInt32Array::from(node_indices));
    let lod_tri_arr: ArrayRef = Arc::new(UInt32Array::from(lod_tri_counts));
    let err_arr: ArrayRef = Arc::new(Float32Array::from(error_metrics));
    let bbox_min_arr: ArrayRef = Arc::new(bbox_min_builder.finish());
    let bbox_max_arr: ArrayRef = Arc::new(bbox_max_builder.finish());
    let sphere_center_arr: ArrayRef = Arc::new(sphere_center_builder.finish());
    let sphere_radius_arr: ArrayRef = Arc::new(Float32Array::from(sphere_radii));

    let schema = Arc::new(Schema::new(vec![
        Field::new("geo_hash", DataType::Utf8, true),
        Field::new("geo_index", DataType::UInt32, false),
        Field::new("nouns", DataType::Utf8, true),
        Field::new("vertex_count", DataType::UInt32, false),
        Field::new("triangle_count", DataType::UInt32, false),
        Field::new("lod_level", DataType::UInt32, false),
        Field::new("asset_key", DataType::Utf8, true),
        Field::new("mesh_index", DataType::UInt32, false),
        Field::new("node_index", DataType::UInt32, false),
        Field::new("lod_triangle_count", DataType::UInt32, false),
        Field::new("error_metric", DataType::Float32, false),
        Field::new(
            "bbox_min",
            DataType::FixedSizeList(Arc::new(Field::new_list_field(DataType::Float32, false)), 3),
            false,
        ),
        Field::new(
            "bbox_max",
            DataType::FixedSizeList(Arc::new(Field::new_list_field(DataType::Float32, false)), 3),
            false,
        ),
        Field::new(
            "sphere_center",
            DataType::FixedSizeList(Arc::new(Field::new_list_field(DataType::Float32, false)), 3),
            false,
        ),
        Field::new("sphere_radius", DataType::Float32, true),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            geo_hash_arr,
            geo_index_arr,
            nouns_arr,
            vertex_arr,
            tri_arr,
            lod_level_arr,
            asset_arr,
            mesh_arr,
            node_arr,
            lod_tri_arr,
            err_arr,
            bbox_min_arr,
            bbox_max_arr,
            sphere_center_arr,
            sphere_radius_arr,
        ],
    )?;

    let mut file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(&mut file, batch.schema(), None)?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

// ============================================================================
// 占位符函数：修复预先缺失的函数
// ============================================================================

/// 导出指定 dbnos 的 instances.json（按 dbnum 分组）
///
/// 这是一个占位符函数，用于修复预先缺失的函数定义。
/// 内部调用 export_dbnum_instances_json 为每个 dbnum 导出。
pub async fn export_instances_json_for_dbnos(
    dbnos: &[u32],
    _mesh_dir: &Path,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    _verbose: bool,
) -> anyhow::Result<()> {
    for &dbnum in dbnos {
        export_dbnum_instances_json(dbnum, output_dir, db_option.clone(), false, None).await?;
    }
    Ok(())
}

/// 导出指定 refnos 的 instances.json（按 dbnum 分组）
///
/// 这是一个占位符函数，用于修复预先缺失的函数定义。
/// 内部调用 export_dbnum_instances_json 为每个 dbnum 导出。
pub async fn export_instances_json_for_refnos_grouped_by_dbno(
    refnos: &[RefnoEnum],
    _mesh_dir: &Path,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    _verbose: bool,
) -> anyhow::Result<()> {
    // 从 refnos 中提取所有唯一的 dbnum
    use std::collections::HashSet;
    let mut dbnos: HashSet<u32> = HashSet::new();
    for refno in refnos {
        if let Some(dbnum) = refno.to_string().split_once('_').and_then(|(db, _)| db.parse::<u32>().ok()) {
            dbnos.insert(dbnum);
        }
    }

    // 为每个 dbnum 导出
    for dbnum in dbnos {
        export_dbnum_instances_json(dbnum, output_dir, db_option.clone(), false, None).await?;
    }
    Ok(())
}

/// 导出指定 refnos 的 instances.json（按 dbnum 分组，合并追加模式）
///
/// 这是一个占位符函数，用于修复预先缺失的函数定义。
/// 目前与 export_instances_json_for_refnos_grouped_by_dbno 行为相同。
pub async fn export_instances_json_for_refnos_grouped_by_dbno_merge(
    refnos: &[RefnoEnum],
    mesh_dir: &Path,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
) -> anyhow::Result<()> {
    export_instances_json_for_refnos_grouped_by_dbno(refnos, mesh_dir, output_dir, db_option, verbose).await
}
