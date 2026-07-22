//! dbnum 级实例导出 Parquet
//!
//! 从 SurrealDB 读取 inst_relate / geo_relate / tubi_relate / trans / aabb 数据，
//! 生成多表 Parquet 文件组，供前端直接查询。
//!
//! 输出表（按 dbnum 分目录，文件名固定）：
//! - `instances.parquet`     — 一行一个实例 refno
//! - `ptsets.parquet`        — 一行一个 cata_hash 局部坐标系关键点
//! - `primitive_keypoints.parquet` — 一行一个 geo_hash 局部坐标系基础几何关键点
//! - `geo_instances.parquet` — 一行一个几何引用 (refno × geo_index)
//! - `tubings.parquet`       — 一行一个 TUBI 段
//! - `transforms.parquet`    — 一行一个唯一 trans_hash
//! - `aabb.parquet`          — 一行一个唯一 aabb_hash
//! - `manifest.json`         — 元信息

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aios_core::SurrealQueryExt;
use aios_core::options::DbOption;
use aios_core::pdms_types::RefnoEnum;
use aios_core::shape::pdms_shape::RsVec3;
use anyhow::{Context, Result};
use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int32Array, RecordBatch, StringArray, UInt32Array,
    UInt64Array,
};
use arrow_schema::{DataType, Field, Schema};
use chrono::{SecondsFormat, Utc};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;
use serde_json::json;
use std::str::FromStr;

// 注: trans/aabb 查询在本模块内自行实现（避免跨模块耦合）
// specs/023 M2：层级/元信息读取统一走 HierView（pe_owner 快照默认 / .tree 回退）
use crate::fast_model::gen_model::hier_view::HierView;
use crate::fast_model::gen_model::model_record_id::{model_refno_id, model_refno_range};
use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use crate::fast_model::gen_model::utilities::is_valid_cata_hash;
use crate::fast_model::unit_converter::{LengthUnit, UnitConverter};

// =============================================================================

pub fn publish_dbnum_latest_manifest(
    artifact_dir: &Path,
    latest_root: &Path,
    dbnum: u32,
) -> Result<PathBuf> {
    let local_manifest_path = artifact_dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&local_manifest_path).with_context(|| {
            format!("读取本地 manifest 失败: {}", local_manifest_path.display())
        })?)?;
    anyhow::ensure!(
        manifest.get("dbnum").and_then(serde_json::Value::as_u64) == Some(u64::from(dbnum)),
        "本地 manifest dbnum 与 latest pointer 不匹配"
    );
    anyhow::ensure!(
        manifest
            .get("root_refno")
            .is_none_or(serde_json::Value::is_null),
        "root-scoped 模型不得发布为 dbnum latest"
    );

    let relative_dir = artifact_dir.strip_prefix(latest_root).with_context(|| {
        format!(
            "artifact 目录不在 latest 根目录内: artifact={} root={}",
            artifact_dir.display(),
            latest_root.display()
        )
    })?;
    anyhow::ensure!(
        !relative_dir.as_os_str().is_empty(),
        "artifact 目录不能等于 latest 根目录"
    );
    let prefix = relative_dir.to_string_lossy().replace('\\', "/");

    let tables = manifest
        .get_mut("tables")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("本地 manifest 缺少 tables"))?;
    anyhow::ensure!(!tables.is_empty(), "本地 manifest 的 tables 不能为空");
    for table in tables.values_mut() {
        let expected_rows = table
            .get("rows")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("manifest table rows 必须是非负整数"))?;
        let file = table
            .get_mut("file")
            .ok_or_else(|| anyhow::anyhow!("manifest table 缺少 file"))?;
        let local_file = file
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("manifest table file 必须是字符串"))?;
        let local_path = Path::new(local_file);
        anyhow::ensure!(
            !local_path.is_absolute()
                && local_path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "manifest table file 必须是安全相对路径: {local_file}"
        );
        anyhow::ensure!(
            artifact_dir.join(local_path).is_file(),
            "manifest 引用文件不存在: {}",
            artifact_dir.join(local_path).display()
        );
        let parquet_file = fs::File::open(artifact_dir.join(local_path))?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(parquet_file).with_context(|| {
            format!(
                "读取 Parquet schema 失败: {}",
                artifact_dir.join(local_path).display()
            )
        })?;
        anyhow::ensure!(
            !reader.schema().fields().is_empty(),
            "Parquet schema 不能为空: {}",
            artifact_dir.join(local_path).display()
        );
        let actual_rows = u64::try_from(reader.metadata().file_metadata().num_rows())
            .context("Parquet row count 不能为负数")?;
        anyhow::ensure!(
            actual_rows == expected_rows,
            "Parquet 行数与 manifest 不一致: file={} expected={} actual={}",
            artifact_dir.join(local_path).display(),
            expected_rows,
            actual_rows
        );
        *file = json!(format!("{prefix}/{local_file}"));
    }

    if let Some(report_file) = manifest
        .pointer_mut("/mesh_validation/report_file")
        .and_then(|value| value.as_str().map(str::to_string))
    {
        let report_path = Path::new(&report_file);
        anyhow::ensure!(
            !report_path.is_absolute()
                && report_path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "mesh report 必须是安全相对路径: {report_file}"
        );
        anyhow::ensure!(
            artifact_dir.join(report_path).is_file(),
            "mesh report 不存在: {}",
            artifact_dir.join(report_path).display()
        );
        *manifest
            .pointer_mut("/mesh_validation/report_file")
            .expect("report_file exists") = json!(format!("{prefix}/{report_file}"));
    }

    fs::create_dir_all(latest_root)?;
    let pointer_path = latest_root.join(format!("manifest_{dbnum}.json"));
    let mut temp = tempfile::NamedTempFile::new_in(latest_root)?;
    serde_json::to_writer_pretty(temp.as_file_mut(), &manifest)?;
    temp.as_file_mut().write_all(b"\n")?;
    temp.as_file_mut().sync_all()?;
    temp.persist(&pointer_path).map_err(|error| error.error)?;
    Ok(pointer_path)
}
// Parquet 行结构体
// =============================================================================

/// instances.parquet 的一行
struct InstanceRow {
    refno_str: String,
    refno_u64: u64,
    noun: String,
    owner_refno_str: Option<String>,
    owner_refno_u64: Option<u64>,
    owner_noun: String,
    cata_hash: Option<String>,
    trans_hash: String,
    aabb_hash: String,
    spec_value: i64,
    spec_info_fallback: bool,
    has_neg: bool,
    dbnum: u32,
}

/// geo_instances.parquet 的一行
struct GeoInstanceRow {
    refno_str: String,
    refno_u64: u64,
    geo_index: u32,
    geo_hash: String,
    geo_trans_hash: String,
}

/// tubings.parquet 的一行
struct TubingRow {
    tubi_refno_str: String,
    tubi_refno_u64: u64,
    owner_refno_str: String,
    owner_refno_u64: u64,
    order: u32,
    geo_hash: String,
    trans_hash: String,
    aabb_hash: String,
    spec_value: i64,
    spec_info_fallback: bool,
    dbnum: u32,
}

/// transforms.parquet 的一行
struct TransformRow {
    trans_hash: String,
    m00: f64,
    m10: f64,
    m20: f64,
    m30: f64,
    m01: f64,
    m11: f64,
    m21: f64,
    m31: f64,
    m02: f64,
    m12: f64,
    m22: f64,
    m32: f64,
    m03: f64,
    m13: f64,
    m23: f64,
    m33: f64,
}

/// aabb.parquet 的一行
struct AabbRow {
    aabb_hash: String,
    min_x: f64,
    min_y: f64,
    min_z: f64,
    max_x: f64,
    max_y: f64,
    max_z: f64,
}

/// ptsets.parquet 的一行：按 cata_hash 复用局部坐标系关键点定义。
#[derive(Clone)]
struct PtsetRow {
    cata_hash: String,
    point_number: i32,
    pt_x: f64,
    pt_y: f64,
    pt_z: f64,
    has_dir: bool,
    dir_x: f64,
    dir_y: f64,
    dir_z: f64,
    dir_flag: f64,
    has_ref_dir: bool,
    ref_dir_x: f64,
    ref_dir_y: f64,
    ref_dir_z: f64,
    pbore: f64,
    pwidth: f64,
    pheight: f64,
    pconnect: String,
}

/// primitive_keypoints.parquet 的一行：按 geo_hash 复用局部坐标系基础几何关键点定义。
#[derive(Clone)]
struct PrimitiveKeyPointRow {
    geo_hash: String,
    keypoint_index: i32,
    kind: String,
    local_x: f64,
    local_y: f64,
    local_z: f64,
    has_dir: bool,
    dir_x: f64,
    dir_y: f64,
    dir_z: f64,
    source: String,
}

// =============================================================================
// 辅助函数
// =============================================================================

fn refno_to_u64(r: &RefnoEnum) -> u64 {
    *r.refno()
}

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(
            parquet::basic::ZstdLevel::try_new(3).unwrap(),
        ))
        .build()
}

fn write_parquet(path: &Path, batch: &RecordBatch) -> Result<u64> {
    let file = fs::File::create(path)
        .with_context(|| format!("创建 Parquet 文件失败: {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(writer_props()))?;
    writer.write(batch)?;
    writer.close()?;
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(size)
}

const MESH_CHECK_LOD_TAG: &str = "L1";
const MESH_REPORT_REFNO_SAMPLE_LIMIT: usize = 50;
const SURREAL_REFNO_QUERY_BATCH_SIZE: usize = 100;
const PRIMITIVE_KEYPOINT_QUERY_BATCH_SIZE: usize = 50;
const MAX_SPEC_ANCESTOR_DEPTH: usize = 64;

struct MissingMeshReportSummary {
    report_file: String,
    checked_geo_hashes: usize,
    missing_geo_hashes: usize,
    missing_owner_refnos: usize,
    missing_geo_hash_values: HashSet<String>,
}

struct MeshValidationExportSummary {
    policy: &'static str,
    raw_checked_geo_hashes: usize,
    raw_missing_geo_hashes: usize,
    raw_missing_owner_refnos: usize,
    render_missing_geo_hashes: usize,
    render_missing_owner_refnos: usize,
    quarantined_geo_hashes: usize,
    quarantined_owner_refnos: usize,
    dropped_geo_instance_rows: usize,
    dropped_tubing_rows: usize,
    dropped_instance_rows: usize,
}

fn mesh_base_dir_from_db_option(db_option: &DbOption) -> PathBuf {
    db_option
        .meshes_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("assets/meshes"))
}

fn normalize_mesh_base_dir(mesh_dir: &Path) -> PathBuf {
    let is_lod_dir = mesh_dir
        .file_name()
        .map(|n| n.to_string_lossy().starts_with("lod_"))
        .unwrap_or(false);
    if is_lod_dir {
        mesh_dir.parent().unwrap_or(mesh_dir).to_path_buf()
    } else {
        mesh_dir.to_path_buf()
    }
}

fn mesh_candidates_for_geo_hash(
    mesh_base_dir: &Path,
    geo_hash: &str,
    lod_tag: &str,
) -> [PathBuf; 3] {
    let lod_dir = mesh_base_dir.join(format!("lod_{}", lod_tag));
    [
        lod_dir.join(format!("{}_{}.glb", geo_hash, lod_tag)),
        lod_dir.join(format!("{}.glb", geo_hash)),
        mesh_base_dir.join(format!("{}.glb", geo_hash)),
    ]
}

fn is_builtin_geo_hash(geo_hash: &str) -> bool {
    matches!(geo_hash.trim(), "0" | "1" | "2" | "3")
}

fn record_geo_hash_usage(
    geo_hash: &str,
    owner_refno: &str,
    owner_refnos_by_hash: &mut HashMap<String, HashSet<String>>,
    row_count_by_hash: &mut HashMap<String, usize>,
) {
    let hash = geo_hash.trim();
    if hash.is_empty() || owner_refno.trim().is_empty() {
        return;
    }
    owner_refnos_by_hash
        .entry(hash.to_string())
        .or_default()
        .insert(owner_refno.to_string());
    *row_count_by_hash.entry(hash.to_string()).or_insert(0) += 1;
}

fn tree_owner_refno(tree_manager: &HierView, refno: RefnoEnum) -> Option<RefnoEnum> {
    let meta = tree_manager.get_node_meta(refno)?;
    (meta.owner.0 != 0).then(|| RefnoEnum::from(meta.owner))
}

fn resolve_spec_value_with_ancestors(
    refno: RefnoEnum,
    owner_refno: Option<RefnoEnum>,
    tree_manager: &HierView,
    spec_info_map: &HashMap<u64, i64>,
    cache: &mut HashMap<u64, i64>,
) -> i64 {
    let original_u64 = refno_to_u64(&refno);
    if let Some(value) = cache.get(&original_u64) {
        return *value;
    }

    let mut current = Some(refno);
    for depth in 0..MAX_SPEC_ANCESTOR_DEPTH {
        let Some(candidate) = current else {
            break;
        };
        let candidate_u64 = refno_to_u64(&candidate);
        if let Some(value) = spec_info_map
            .get(&candidate_u64)
            .copied()
            .filter(|value| *value != 0)
        {
            cache.insert(original_u64, value);
            return value;
        }

        let parent = if depth == 0 {
            owner_refno.or_else(|| tree_owner_refno(tree_manager, candidate))
        } else {
            tree_owner_refno(tree_manager, candidate)
        };
        if parent.map(|p| refno_to_u64(&p)) == Some(candidate_u64) {
            break;
        }
        current = parent;
    }

    cache.insert(original_u64, 0);
    0
}

fn pe_record_list(refnos: &[RefnoEnum]) -> String {
    refnos
        .iter()
        .map(|refno| refno.to_pe_key())
        .collect::<Vec<_>>()
        .join(", ")
}

fn append_tubing_rows_for_owner(
    owner_refno: &RefnoEnum,
    tubis: &[TubiQueryResult],
    spec_info_map: &HashMap<u64, i64>,
    tree_manager: &HierView,
    spec_lookup_cache: &mut HashMap<u64, i64>,
    dbnum: u32,
    tubing_rows: &mut Vec<TubingRow>,
    trans_hashes: &mut HashSet<String>,
    aabb_hashes: &mut HashSet<String>,
    owner_refnos_by_hash: &mut HashMap<String, HashSet<String>>,
    row_count_by_hash: &mut HashMap<String, usize>,
) {
    for tubi in tubis {
        let aabb_hash = tubi.world_aabb_hash.clone().unwrap_or_default();
        let trans_hash = tubi.world_trans_hash.clone().unwrap_or_default();
        let geo_hash = tubi.geo_hash.clone().unwrap_or_default();

        if aabb_hash.is_empty() || geo_hash.is_empty() {
            continue;
        }

        if !aabb_hash.is_empty() {
            aabb_hashes.insert(aabb_hash.clone());
        }
        if !trans_hash.is_empty() {
            trans_hashes.insert(trans_hash.clone());
        }

        let index = tubi.index.and_then(|v| u32::try_from(v).ok()).unwrap_or(0);

        let mut tubi_spec = tubi.spec_value.unwrap_or(0);
        let mut spec_info_fallback = false;
        if tubi_spec == 0 {
            tubi_spec = resolve_spec_value_with_ancestors(
                tubi.leave,
                Some(*owner_refno),
                tree_manager,
                spec_info_map,
                spec_lookup_cache,
            );
            spec_info_fallback = tubi_spec == 0;
        }

        tubing_rows.push(TubingRow {
            tubi_refno_str: tubi.leave.to_string(),
            tubi_refno_u64: refno_to_u64(&tubi.leave),
            owner_refno_str: owner_refno.to_string(),
            owner_refno_u64: refno_to_u64(owner_refno),
            order: index,
            geo_hash,
            trans_hash,
            aabb_hash,
            spec_value: tubi_spec,
            spec_info_fallback,
            dbnum,
        });
        record_geo_hash_usage(
            &tubi.geo_hash.clone().unwrap_or_default(),
            &owner_refno.to_string(),
            owner_refnos_by_hash,
            row_count_by_hash,
        );
    }
}

fn write_missing_mesh_report(
    output_dir: &Path,
    dbnum: u32,
    mesh_base_dir: &Path,
    lod_tag: &str,
    owner_refnos_by_hash: &HashMap<String, HashSet<String>>,
    row_count_by_hash: &HashMap<String, usize>,
    verbose: bool,
) -> Result<MissingMeshReportSummary> {
    let mut checked_geo_hashes = 0usize;
    let mut missing_owner_union: HashSet<String> = HashSet::new();
    let mut missing_entries: Vec<(String, usize, usize, Vec<String>, Vec<String>)> = Vec::new();

    for geo_hash in owner_refnos_by_hash.keys() {
        let hash = geo_hash.trim();
        if hash.is_empty() || is_builtin_geo_hash(hash) {
            continue;
        }
        checked_geo_hashes += 1;

        let candidates = mesh_candidates_for_geo_hash(mesh_base_dir, hash, lod_tag);
        let exists = candidates.iter().any(|p| p.exists());
        if exists {
            continue;
        }

        let mut owners = owner_refnos_by_hash
            .get(hash)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        owners.sort();
        for r in &owners {
            missing_owner_union.insert(r.clone());
        }
        let owner_sample = owners
            .iter()
            .take(MESH_REPORT_REFNO_SAMPLE_LIMIT)
            .cloned()
            .collect::<Vec<_>>();
        let row_count = *row_count_by_hash.get(hash).unwrap_or(&0);
        let candidate_paths = candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        missing_entries.push((
            hash.to_string(),
            row_count,
            owners.len(),
            owner_sample,
            candidate_paths,
        ));
    }

    missing_entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let missing_geo_hashes_json = missing_entries
        .iter()
        .map(
            |(geo_hash, row_count, owner_count, owner_sample, candidate_paths)| {
                json!({
                    "geo_hash": geo_hash,
                    "row_count": row_count,
                    "owner_refno_count": owner_count,
                    "owner_refno_sample": owner_sample,
                    "owner_refno_sample_count": owner_sample.len(),
                    "mesh_candidates": candidate_paths,
                })
            },
        )
        .collect::<Vec<_>>();

    let report = json!({
        "version": 1,
        "generated_at": generated_at,
        "dbnum": dbnum,
        "mesh_base_dir": mesh_base_dir.display().to_string(),
        "lod_tag": lod_tag,
        "checked_geo_hashes": checked_geo_hashes,
        "missing_geo_hashes": missing_entries.len(),
        "missing_owner_refnos": missing_owner_union.len(),
        "missing_geo_hash_list": missing_geo_hashes_json,
    });

    let report_file = format!("missing_mesh_report_{}.json", dbnum);
    let report_path = output_dir.join(&report_file);
    fs::write(&report_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("写入缺失 mesh 报告失败: {}", report_path.display()))?;

    if !missing_entries.is_empty() {
        eprintln!(
            "[parquet] dbnum={} 检测到缺失 mesh: geo_hashes={}, owner_refnos={}，报告={}",
            dbnum,
            missing_entries.len(),
            missing_owner_union.len(),
            report_path.display()
        );
    } else if verbose {
        println!(
            "   ✅ mesh 校验通过: checked_geo_hashes={} (lod={})",
            checked_geo_hashes, lod_tag
        );
    }

    Ok(MissingMeshReportSummary {
        report_file,
        checked_geo_hashes,
        missing_geo_hashes: missing_entries.len(),
        missing_owner_refnos: missing_owner_union.len(),
        missing_geo_hash_values: missing_entries
            .iter()
            .map(|(geo_hash, _, _, _, _)| geo_hash.clone())
            .collect(),
    })
}

// =============================================================================
// Schema 定义
// =============================================================================

fn instances_schema() -> Schema {
    Schema::new(vec![
        Field::new("refno_str", DataType::Utf8, false),
        Field::new("refno_u64", DataType::UInt64, false),
        Field::new("noun", DataType::Utf8, false),
        Field::new("owner_refno_str", DataType::Utf8, true),
        Field::new("owner_refno_u64", DataType::UInt64, true),
        Field::new("owner_noun", DataType::Utf8, false),
        Field::new("cata_hash", DataType::Utf8, true),
        Field::new("trans_hash", DataType::Utf8, false),
        Field::new("aabb_hash", DataType::Utf8, false),
        Field::new("spec_value", DataType::UInt64, false),
        Field::new("has_neg", DataType::Boolean, false),
        Field::new("dbnum", DataType::UInt32, false),
    ])
}

fn geo_instances_schema() -> Schema {
    Schema::new(vec![
        Field::new("refno_str", DataType::Utf8, false),
        Field::new("refno_u64", DataType::UInt64, false),
        Field::new("geo_index", DataType::UInt32, false),
        Field::new("geo_hash", DataType::Utf8, false),
        Field::new("geo_trans_hash", DataType::Utf8, false),
    ])
}

fn tubings_schema() -> Schema {
    Schema::new(vec![
        Field::new("tubi_refno_str", DataType::Utf8, false),
        Field::new("tubi_refno_u64", DataType::UInt64, false),
        Field::new("owner_refno_str", DataType::Utf8, false),
        Field::new("owner_refno_u64", DataType::UInt64, false),
        Field::new("order", DataType::UInt32, false),
        Field::new("geo_hash", DataType::Utf8, false),
        Field::new("trans_hash", DataType::Utf8, false),
        Field::new("aabb_hash", DataType::Utf8, false),
        Field::new("spec_value", DataType::UInt64, false),
        Field::new("dbnum", DataType::UInt32, false),
    ])
}

fn transforms_schema() -> Schema {
    Schema::new(vec![
        Field::new("trans_hash", DataType::Utf8, false),
        Field::new("m00", DataType::Float64, false),
        Field::new("m10", DataType::Float64, false),
        Field::new("m20", DataType::Float64, false),
        Field::new("m30", DataType::Float64, false),
        Field::new("m01", DataType::Float64, false),
        Field::new("m11", DataType::Float64, false),
        Field::new("m21", DataType::Float64, false),
        Field::new("m31", DataType::Float64, false),
        Field::new("m02", DataType::Float64, false),
        Field::new("m12", DataType::Float64, false),
        Field::new("m22", DataType::Float64, false),
        Field::new("m32", DataType::Float64, false),
        Field::new("m03", DataType::Float64, false),
        Field::new("m13", DataType::Float64, false),
        Field::new("m23", DataType::Float64, false),
        Field::new("m33", DataType::Float64, false),
    ])
}

fn aabb_schema() -> Schema {
    Schema::new(vec![
        Field::new("aabb_hash", DataType::Utf8, false),
        Field::new("min_x", DataType::Float64, false),
        Field::new("min_y", DataType::Float64, false),
        Field::new("min_z", DataType::Float64, false),
        Field::new("max_x", DataType::Float64, false),
        Field::new("max_y", DataType::Float64, false),
        Field::new("max_z", DataType::Float64, false),
    ])
}

fn ptsets_schema() -> Schema {
    Schema::new(vec![
        Field::new("cata_hash", DataType::Utf8, false),
        Field::new("point_number", DataType::Int32, false),
        Field::new("pt_x", DataType::Float64, false),
        Field::new("pt_y", DataType::Float64, false),
        Field::new("pt_z", DataType::Float64, false),
        Field::new("has_dir", DataType::Boolean, false),
        Field::new("dir_x", DataType::Float64, false),
        Field::new("dir_y", DataType::Float64, false),
        Field::new("dir_z", DataType::Float64, false),
        Field::new("dir_flag", DataType::Float64, false),
        Field::new("has_ref_dir", DataType::Boolean, false),
        Field::new("ref_dir_x", DataType::Float64, false),
        Field::new("ref_dir_y", DataType::Float64, false),
        Field::new("ref_dir_z", DataType::Float64, false),
        Field::new("pbore", DataType::Float64, false),
        Field::new("pwidth", DataType::Float64, false),
        Field::new("pheight", DataType::Float64, false),
        Field::new("pconnect", DataType::Utf8, false),
    ])
}

fn primitive_keypoints_schema() -> Schema {
    Schema::new(vec![
        Field::new("geo_hash", DataType::Utf8, false),
        Field::new("keypoint_index", DataType::Int32, false),
        Field::new("kind", DataType::Utf8, false),
        Field::new("local_x", DataType::Float64, false),
        Field::new("local_y", DataType::Float64, false),
        Field::new("local_z", DataType::Float64, false),
        Field::new("has_dir", DataType::Boolean, false),
        Field::new("dir_x", DataType::Float64, false),
        Field::new("dir_y", DataType::Float64, false),
        Field::new("dir_z", DataType::Float64, false),
        Field::new("source", DataType::Utf8, false),
    ])
}

// =============================================================================
// RecordBatch 构建
// =============================================================================

fn build_instances_batch(rows: &[InstanceRow]) -> Result<RecordBatch> {
    let schema = Arc::new(instances_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.refno_str.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.refno_u64).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.noun.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.owner_refno_str.as_deref())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter()
                    .map(|r| r.owner_refno_u64)
                    .collect::<Vec<Option<u64>>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.owner_noun.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.cata_hash.as_deref())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.trans_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.aabb_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.spec_value as u64).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(BooleanArray::from(
                rows.iter().map(|r| r.has_neg).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.dbnum).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_ptsets_batch(rows: &[PtsetRow]) -> Result<RecordBatch> {
    let schema = Arc::new(ptsets_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.cata_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Int32Array::from(
                rows.iter().map(|r| r.point_number).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.pt_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.pt_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.pt_z).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(BooleanArray::from(
                rows.iter().map(|r| r.has_dir).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_z).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_flag).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(BooleanArray::from(
                rows.iter().map(|r| r.has_ref_dir).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.ref_dir_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.ref_dir_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.ref_dir_z).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.pbore).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.pwidth).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.pheight).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.pconnect.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_primitive_keypoints_batch(rows: &[PrimitiveKeyPointRow]) -> Result<RecordBatch> {
    let schema = Arc::new(primitive_keypoints_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.geo_hash.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Int32Array::from(
                rows.iter().map(|r| r.keypoint_index).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.local_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.local_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.local_z).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(BooleanArray::from(
                rows.iter().map(|r| r.has_dir).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.dir_z).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.source.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_geo_instances_batch(rows: &[GeoInstanceRow]) -> Result<RecordBatch> {
    let schema = Arc::new(geo_instances_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.refno_str.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.refno_u64).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.geo_index).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.geo_hash.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.geo_trans_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_tubings_batch(rows: &[TubingRow]) -> Result<RecordBatch> {
    let schema = Arc::new(tubings_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.tubi_refno_str.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.tubi_refno_u64).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.owner_refno_str.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.owner_refno_u64).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.order).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter().map(|r| r.geo_hash.as_str()).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.trans_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.aabb_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt64Array::from(
                rows.iter().map(|r| r.spec_value as u64).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(UInt32Array::from(
                rows.iter().map(|r| r.dbnum).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_transforms_batch(rows: &[TransformRow]) -> Result<RecordBatch> {
    let schema = Arc::new(transforms_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.trans_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m00).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m10).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m20).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m30).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m01).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m11).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m21).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m31).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m02).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m12).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m22).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m32).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m03).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m13).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m23).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.m33).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn build_aabb_batch(rows: &[AabbRow]) -> Result<RecordBatch> {
    let schema = Arc::new(aabb_schema());
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.aabb_hash.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.min_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.min_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.min_z).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.max_x).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.max_y).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                rows.iter().map(|r| r.max_z).collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )?;
    Ok(batch)
}

fn append_owner_chain_rows(
    instance_rows: &mut Vec<InstanceRow>,
    tree_manager: &HierView,
    spec_info_map: &HashMap<u64, i64>,
    spec_lookup_cache: &mut HashMap<u64, i64>,
    dbnum: u32,
    root_refno: Option<RefnoEnum>,
    verbose: bool,
) {
    let mut present_refnos = instance_rows
        .iter()
        .map(|row| row.refno_u64)
        .collect::<HashSet<_>>();
    let mut pending = instance_rows
        .iter()
        .filter_map(|row| row.owner_refno_u64)
        .collect::<Vec<_>>();
    let mut appended = 0usize;

    while let Some(owner_u64) = pending.pop() {
        if present_refnos.contains(&owner_u64) {
            continue;
        }

        let owner_refno = RefnoEnum::from(aios_core::RefU64(owner_u64));
        let Some(meta) = tree_manager.get_node_meta(owner_refno) else {
            continue;
        };
        let parent_refno = if meta.owner.0 == 0 {
            None
        } else {
            Some(RefnoEnum::from(meta.owner))
        };
        let parent_u64 = parent_refno.as_ref().map(refno_to_u64);
        let noun = tree_manager.get_noun(owner_refno).unwrap_or_default();
        let owner_noun = parent_refno
            .and_then(|parent| tree_manager.get_noun(parent))
            .unwrap_or_default();
        let spec_value = resolve_spec_value_with_ancestors(
            owner_refno,
            parent_refno,
            tree_manager,
            spec_info_map,
            spec_lookup_cache,
        );

        present_refnos.insert(owner_u64);
        if let Some(parent_u64) = parent_u64 {
            if root_refno.map(|root| refno_to_u64(&root)) != Some(owner_u64) {
                pending.push(parent_u64);
            }
        }

        instance_rows.push(InstanceRow {
            refno_str: owner_refno.to_string(),
            refno_u64: owner_u64,
            noun,
            owner_refno_str: parent_refno.map(|r| r.to_string()),
            owner_refno_u64: parent_u64,
            owner_noun,
            cata_hash: None,
            trans_hash: String::new(),
            aabb_hash: String::new(),
            spec_value,
            spec_info_fallback: spec_value == 0,
            has_neg: false,
            dbnum,
        });
        appended += 1;
    }

    if verbose && appended > 0 {
        println!(
            "   ✅ instances.parquet 补齐 owner 链语义节点: {} 行",
            appended
        );
    }
}

// =============================================================================
// SurrealDB 查询结构体
// =============================================================================

use aios_core::parsed_data::CateAxisParam;
use aios_core::vec3_pool::{CateAxisParamCompact, decompress_ptset};
use serde::{Deserialize, Serialize};
use surrealdb::types::{self as surrealdb_types, SurrealValue};

// InstRelateRow 使用 export_common 中的共享定义
use super::InstRelateRow;

#[derive(Clone, Debug, Serialize, Deserialize, SurrealValue)]
struct TubiQueryResult {
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

#[derive(Debug, Deserialize, SurrealValue)]
struct InstInfoPtsetQueryRow {
    refno: RefnoEnum,
    inst_info_id: Option<serde_json::Value>,
    cata_hash: Option<serde_json::Value>,
    ptset: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct InstInfoPtsetRecordRow {
    inst_info_id: Option<serde_json::Value>,
    cata_hash: Option<serde_json::Value>,
    ptset: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PrimitiveKeyPointQueryRow {
    geo_hash: String,
    pts: Option<Vec<Option<aios_core::shape::pdms_shape::RsVec3>>>,
}

struct PtsetExportData {
    refno_cata_hash: HashMap<RefnoEnum, String>,
    rows_by_cata_hash: HashMap<String, Vec<PtsetRow>>,
    requested_refnos: usize,
    relation_rows: usize,
    inst_info_rows: usize,
    invalid_cata_hash_rows: usize,
    missing_cata_hash_refnos: usize,
    empty_ptset_hashes: usize,
}

fn normalize_record_id(value: Option<serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => {
            let trimmed = s.trim().trim_matches('⟨').trim_matches('⟩').to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Array(mut values) if values.len() == 1 => {
            normalize_record_id(Some(values.remove(0)))
        }
        _ => None,
    }
}

fn normalize_cata_hash_str(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_matches('⟨').trim_matches('⟩').to_string();
    is_valid_cata_hash(&trimmed).then_some(trimmed)
}

fn normalize_cata_hash(value: Option<serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => normalize_cata_hash_str(&s),
        serde_json::Value::Number(n) => normalize_cata_hash_str(&n.to_string()),
        serde_json::Value::Array(values) => values
            .into_iter()
            .find_map(|value| normalize_cata_hash(Some(value))),
        _ => None,
    }
}

fn normalize_cata_hash_record_id(value: Option<serde_json::Value>) -> Option<String> {
    normalize_record_id(value).and_then(|id| normalize_cata_hash_str(&id))
}

fn inst_info_record_ref(id: &str) -> String {
    format!(
        "inst_info:⟨{}⟩",
        id.trim().trim_matches('⟨').trim_matches('⟩')
    )
}

fn synthetic_ptset_lookup_key(refno: RefnoEnum) -> String {
    // ponytail: keep the Parquet schema stable; this string is a PTSET lookup key, not a catalog hash.
    format!("refno:{refno}")
}

fn parse_vec3_value(value: &serde_json::Value) -> Option<glam::Vec3> {
    if let Some(values) = value.as_array() {
        let x = values.first()?.as_f64()? as f32;
        let y = values.get(1)?.as_f64()? as f32;
        let z = values.get(2)?.as_f64()? as f32;
        return Some(glam::Vec3::new(x, y, z));
    }

    let obj = value.as_object()?;
    if let Some(inner) = obj.get("d") {
        return parse_vec3_value(inner);
    }
    if let (Some(x), Some(y), Some(z)) = (obj.get("x"), obj.get("y"), obj.get("z")) {
        return Some(glam::Vec3::new(
            x.as_f64()? as f32,
            y.as_f64()? as f32,
            z.as_f64()? as f32,
        ));
    }
    None
}

fn parse_refno_value(value: Option<&serde_json::Value>) -> RefnoEnum {
    let Some(value) = value else {
        return RefnoEnum::default();
    };

    let raw = value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_u64().map(|v| v.to_string()))
        .unwrap_or_default();
    let normalized = raw
        .trim()
        .trim_start_matches("pe:")
        .trim_matches('⟨')
        .trim_matches('⟩')
        .replace('_', "/");
    RefnoEnum::from_str(&normalized).unwrap_or_default()
}

fn parse_optional_vec3(value: Option<&serde_json::Value>) -> Option<RsVec3> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    parse_vec3_value(value).map(RsVec3)
}

fn parse_ptset_axis_object(value: &serde_json::Value) -> Option<CateAxisParam> {
    let obj = value.as_object()?;
    let number = obj.get("number").and_then(|v| v.as_i64())? as i32;
    let pt = parse_vec3_value(obj.get("pt")?)?;
    Some(CateAxisParam {
        refno: parse_refno_value(obj.get("refno")),
        number,
        pt: RsVec3(pt),
        dir: parse_optional_vec3(obj.get("dir")),
        dir_flag: obj.get("dir_flag").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
        ref_dir: parse_optional_vec3(obj.get("ref_dir")),
        pbore: obj.get("pbore").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        pwidth: obj.get("pwidth").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        pheight: obj.get("pheight").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        pconnect: obj
            .get("pconnect")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    })
}

fn parse_ptset_value(value: serde_json::Value) -> Vec<CateAxisParam> {
    let value = match value {
        serde_json::Value::Array(mut values) if values.len() == 1 && values[0].is_array() => {
            values.remove(0)
        }
        value => value,
    };

    match serde_json::from_value::<Vec<CateAxisParamCompact>>(value.clone()) {
        Ok(compact) => decompress_ptset(&compact),
        Err(_) => {
            serde_json::from_value::<Vec<CateAxisParam>>(value.clone()).unwrap_or_else(|_| {
                value
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(parse_ptset_axis_object)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
        }
    }
}

fn axis_to_ptset_row(cata_hash: &str, axis: &CateAxisParam) -> PtsetRow {
    let dir = axis.dir.as_ref().map(|v| v.0);
    let ref_dir = axis.ref_dir.as_ref().map(|v| v.0);
    PtsetRow {
        cata_hash: cata_hash.to_string(),
        point_number: axis.number,
        pt_x: axis.pt.0.x as f64,
        pt_y: axis.pt.0.y as f64,
        pt_z: axis.pt.0.z as f64,
        has_dir: dir.is_some(),
        dir_x: dir.map(|v| v.x as f64).unwrap_or(0.0),
        dir_y: dir.map(|v| v.y as f64).unwrap_or(0.0),
        dir_z: dir.map(|v| v.z as f64).unwrap_or(0.0),
        dir_flag: axis.dir_flag as f64,
        has_ref_dir: ref_dir.is_some(),
        ref_dir_x: ref_dir.map(|v| v.x as f64).unwrap_or(0.0),
        ref_dir_y: ref_dir.map(|v| v.y as f64).unwrap_or(0.0),
        ref_dir_z: ref_dir.map(|v| v.z as f64).unwrap_or(0.0),
        pbore: axis.pbore as f64,
        pwidth: axis.pwidth as f64,
        pheight: axis.pheight as f64,
        pconnect: axis.pconnect.clone(),
    }
}

// =============================================================================
// SurrealDB 查询函数
// =============================================================================

/// 从 inst_relate 查询所有 distinct dbnum（排序后返回）
///
/// 用于 `--export-parquet` 未指定 `--dbnum` 时自动扫描并逐一导出。
pub async fn query_distinct_dbnums_from_inst_relate() -> Result<Vec<u32>> {
    use aios_core::SurrealQueryExt;

    let sql = "SELECT VALUE array::distinct(dbnum) FROM inst_relate GROUP ALL;";
    let result: Vec<Vec<i64>> = aios_core::project_primary_db().query_take(sql, 0).await?;

    let mut dbnums: Vec<u32> = result
        .into_iter()
        .flatten()
        .filter_map(|v| if v >= 0 { Some(v as u32) } else { None })
        .collect();
    dbnums.sort_unstable();
    dbnums.dedup();
    Ok(dbnums)
}

/// 通过 dbnum 字段过滤 inst_relate（命中 dbnum 索引路径）
async fn query_inst_relate_by_dbnum(dbnum: u32, verbose: bool) -> Result<Vec<InstRelateRow>> {
    if verbose {
        println!(
            "🔍 扫描 inst_relate（索引路径: WHERE dbnum = {}）...",
            dbnum
        );
    }

    const PAGE_SIZE: usize = 10_000;

    let query_start = std::time::Instant::now();
    let mut rows = Vec::new();
    let mut offset = 0usize;
    let mut page = 0usize;

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
            WHERE dbnum = $dbnum
            ORDER BY in
            LIMIT {PAGE_SIZE} START {offset}
            "#
        );

        let page_start = std::time::Instant::now();
        let mut resp = aios_core::project_primary_db()
            .query(&sql)
            .bind(("dbnum", dbnum))
            .await?;
        let mut page_rows: Vec<InstRelateRow> = resp.take(0)?;

        if page_rows.is_empty() {
            break;
        }

        page += 1;
        offset += page_rows.len();

        if verbose {
            println!(
                "   - inst_relate page {}: {} rows ({:?})",
                page,
                page_rows.len(),
                page_start.elapsed()
            );
        }

        let is_last_page = page_rows.len() < PAGE_SIZE;
        rows.append(&mut page_rows);

        if is_last_page {
            break;
        }
    }

    if verbose {
        println!(
            "✅ inst_relate 命中记录: {} ({:?})",
            rows.len(),
            query_start.elapsed()
        );
    }

    if rows.is_empty() {
        if verbose {
            println!(
                "⚠️ inst_relate.dbnum 未命中 dbnum={}，回退到 refno->dbnum 缓存过滤...",
                dbnum
            );
        }
        rows = query_inst_relate_by_refno_dbnum_fallback(dbnum, verbose).await?;
    }

    Ok(rows)
}

/// 兼容旧/轻量写入路径：部分 inst_relate 关系未写入 dbnum 字段。
/// 此时通过 db_meta_info/refno 缓存推导所属 dbnum，避免生成后 Parquet 被导成 0 行。
async fn query_inst_relate_by_refno_dbnum_fallback(
    dbnum: u32,
    verbose: bool,
) -> Result<Vec<InstRelateRow>> {
    if verbose {
        println!("🔍 扫描 inst_relate（兼容路径: refno -> dbnum）...");
    }

    const PAGE_SIZE: usize = 10_000;
    let query_start = std::time::Instant::now();
    let mut rows = Vec::new();
    let mut offset = 0usize;
    let mut page = 0usize;

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
            WHERE in != NONE
            ORDER BY in
            LIMIT {PAGE_SIZE} START {offset}
            "#
        );

        let mut resp = aios_core::project_primary_db().query(&sql).await?;
        let page_rows: Vec<InstRelateRow> = resp.take(0)?;
        if page_rows.is_empty() {
            break;
        }

        page += 1;
        offset += page_rows.len();
        let page_len = page_rows.len();
        rows.extend(page_rows.into_iter().filter(|row| {
            TreeIndexManager::resolve_dbnum_for_refno(row.refno)
                .map(|resolved| resolved == dbnum)
                .unwrap_or(false)
        }));

        if verbose {
            println!(
                "   - fallback page {}: scanned={}, matched_total={}",
                page,
                page_len,
                rows.len()
            );
        }

        if page_len < PAGE_SIZE {
            break;
        }
    }

    if verbose {
        println!(
            "✅ fallback inst_relate 命中记录: {} ({:?})",
            rows.len(),
            query_start.elapsed()
        );
    }

    Ok(rows)
}

/// 使用 refno 列表分批查询 inst_relate（root_refno 子树模式使用）
async fn query_inst_relate_by_refnos(
    refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<Vec<InstRelateRow>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows = Vec::new();

    for (idx, chunk) in refnos.chunks(SURREAL_REFNO_QUERY_BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 inst_relate 分批 {}/{} (批大小 {})",
                idx + 1,
                refnos.len().div_ceil(SURREAL_REFNO_QUERY_BATCH_SIZE),
                chunk.len()
            );
        }

        let pe_list = pe_record_list(chunk);

        let sql = format!(
            r#"
            SELECT
                owner_refno,
                owner_type,
                in as refno,
                in.noun as noun,
                spec_value as spec_value
            FROM inst_relate
            WHERE in IN [{pe_list}]
            "#
        );

        let mut chunk_rows: Vec<InstRelateRow> =
            aios_core::project_primary_db().query_take(&sql, 0).await?;
        rows.append(&mut chunk_rows);
    }

    Ok(rows)
}

/// 批量查询实例的 cata_hash 与 ptset，构建 instances/ptsets 共用的导出索引。
async fn query_ptset_export_data(refnos: &[RefnoEnum], verbose: bool) -> Result<PtsetExportData> {
    let mut refno_cata_hash: HashMap<RefnoEnum, String> = HashMap::new();
    let mut rows_by_cata_hash: HashMap<String, Vec<PtsetRow>> = HashMap::new();
    let mut missing_cata_hash_refnos = 0usize;
    let mut empty_ptset_hashes = 0usize;

    if refnos.is_empty() {
        return Ok(PtsetExportData {
            refno_cata_hash,
            rows_by_cata_hash,
            requested_refnos: 0,
            relation_rows: 0,
            inst_info_rows: 0,
            invalid_cata_hash_rows: 0,
            missing_cata_hash_refnos,
            empty_ptset_hashes,
        });
    }

    let requested_refnos = refnos.len();
    let mut relation_rows = 0usize;
    let mut inst_info_rows = 0usize;
    let mut invalid_cata_hash_rows = 0usize;
    let mut cata_hash_inst_info: HashMap<String, String> = HashMap::new();

    for (idx, chunk) in refnos.chunks(SURREAL_REFNO_QUERY_BATCH_SIZE).enumerate() {
        if verbose {
            println!(
                "   - 查询 inst_info ptset 分批 {}/{} (批大小 {})",
                idx + 1,
                refnos.len().div_ceil(SURREAL_REFNO_QUERY_BATCH_SIZE),
                chunk.len()
            );
        }

        let pe_list = pe_record_list(chunk);

        let sql = format!(
            r#"
            SELECT
                in as refno,
                record::id(out) as inst_info_id,
                out.cata_hash as cata_hash,
                out.ptset as ptset
            FROM inst_relate
            WHERE in IN [{pe_list}]
                AND out != NONE
            "#
        );

        let rows: Vec<InstInfoPtsetQueryRow> = aios_core::project_primary_db()
            .query_take(&sql, 0)
            .await
            .with_context(|| format!("query_ptset_export_data SQL: {sql}"))?;

        for row in rows {
            relation_rows += 1;
            let inst_info_id = normalize_record_id(row.inst_info_id);
            let cata_hash = normalize_cata_hash(row.cata_hash)
                .or_else(|| inst_info_id.as_deref().and_then(normalize_cata_hash_str));
            let has_catalog_cata_hash = cata_hash.is_some();
            let ptset_axes = row.ptset.map(parse_ptset_value).unwrap_or_default();
            let cata_hash = match cata_hash {
                Some(cata_hash) => cata_hash,
                None if !ptset_axes.is_empty() => synthetic_ptset_lookup_key(row.refno),
                None => {
                    invalid_cata_hash_rows += 1;
                    continue;
                }
            };
            let inst_info_id = inst_info_id.unwrap_or_else(|| cata_hash.clone());

            refno_cata_hash.insert(row.refno, cata_hash.clone());
            if has_catalog_cata_hash {
                cata_hash_inst_info
                    .entry(cata_hash.clone())
                    .or_insert(inst_info_id);
            }

            if !ptset_axes.is_empty() && !rows_by_cata_hash.contains_key(&cata_hash) {
                let ptset_rows = ptset_axes
                    .iter()
                    .map(|axis| axis_to_ptset_row(&cata_hash, axis))
                    .collect::<Vec<_>>();

                if !ptset_rows.is_empty() {
                    rows_by_cata_hash.insert(cata_hash, ptset_rows);
                }
            }
        }
    }

    let mut missing_ptset_hashes = cata_hash_inst_info
        .iter()
        .filter_map(|(cata_hash, inst_info_id)| {
            rows_by_cata_hash
                .get(cata_hash)
                .map_or(true, Vec::is_empty)
                .then_some((cata_hash.clone(), inst_info_id.clone()))
        })
        .collect::<Vec<_>>();
    missing_ptset_hashes.sort_by(|a, b| a.0.cmp(&b.0));

    for chunk in missing_ptset_hashes.chunks(SURREAL_REFNO_QUERY_BATCH_SIZE) {
        let inst_info_list = chunk
            .iter()
            .map(|(_, inst_info_id)| inst_info_record_ref(inst_info_id))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            r#"
            SELECT
                record::id(id) as inst_info_id,
                cata_hash as cata_hash,
                ptset as ptset
            FROM [{inst_info_list}]
            "#
        );

        let rows: Vec<InstInfoPtsetRecordRow> = aios_core::project_primary_db()
            .query_take(&sql, 0)
            .await
            .with_context(|| format!("query_ptset_export_data inst_info SQL: {sql}"))?;

        for row in rows {
            inst_info_rows += 1;
            let cata_hash = normalize_cata_hash(row.cata_hash)
                .or_else(|| normalize_cata_hash_record_id(row.inst_info_id));
            let Some(cata_hash) = cata_hash else {
                invalid_cata_hash_rows += 1;
                continue;
            };

            if rows_by_cata_hash
                .get(&cata_hash)
                .map_or(false, |rows| !rows.is_empty())
            {
                continue;
            }

            let ptset_rows = row
                .ptset
                .map(parse_ptset_value)
                .unwrap_or_default()
                .iter()
                .map(|axis| axis_to_ptset_row(&cata_hash, axis))
                .collect::<Vec<_>>();

            rows_by_cata_hash.insert(cata_hash, ptset_rows);
        }
    }

    empty_ptset_hashes = rows_by_cata_hash
        .values()
        .filter(|rows| rows.is_empty())
        .count();

    missing_cata_hash_refnos = refnos
        .iter()
        .filter(|refno| !refno_cata_hash.contains_key(refno))
        .count();

    if verbose {
        let ptset_point_count: usize = rows_by_cata_hash.values().map(Vec::len).sum();
        println!(
            "✅ ptset 导出索引: requested_refnos={}, relation_rows={}, inst_info_rows={}, refno_cata_hash={}, cata_hash={}, ptset_points={}, missing_cata_hash_refnos={}, invalid_cata_hash_rows={}, empty_ptset_hashes={}",
            requested_refnos,
            relation_rows,
            inst_info_rows,
            refno_cata_hash.len(),
            rows_by_cata_hash.len(),
            ptset_point_count,
            missing_cata_hash_refnos,
            invalid_cata_hash_rows,
            empty_ptset_hashes
        );
    }

    Ok(PtsetExportData {
        refno_cata_hash,
        rows_by_cata_hash,
        requested_refnos,
        relation_rows,
        inst_info_rows,
        invalid_cata_hash_rows,
        missing_cata_hash_refnos,
        empty_ptset_hashes,
    })
}

/// 批量查询 geo_relate.pts 中的基础几何关键点，按 geo_hash 导出局部坐标模板。
async fn query_primitive_keypoint_rows(
    geo_hashes: &HashSet<String>,
    verbose: bool,
) -> Result<Vec<PrimitiveKeyPointRow>> {
    if geo_hashes.is_empty() {
        return Ok(Vec::new());
    }

    let primitive_keypoints_enabled = std::env::var("AIOS_PARQUET_ENABLE_PRIMITIVE_KEYPOINTS")
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    if !primitive_keypoints_enabled {
        if verbose {
            println!(
                "⚠️ primitive_keypoints.parquet 默认跳过: geo_hashes={}（设置 AIOS_PARQUET_ENABLE_PRIMITIVE_KEYPOINTS=1 可启用慢查询）",
                geo_hashes.len()
            );
        }
        return Ok(Vec::new());
    }

    let mut rows_by_geo_hash: HashMap<String, Vec<PrimitiveKeyPointRow>> = HashMap::new();
    let mut sorted_geo_hashes = geo_hashes.iter().cloned().collect::<Vec<_>>();
    sorted_geo_hashes.sort();

    for chunk in sorted_geo_hashes.chunks(PRIMITIVE_KEYPOINT_QUERY_BATCH_SIZE) {
        match query_primitive_keypoint_rows_chunk(chunk).await {
            Ok(query_rows) => {
                merge_primitive_keypoint_query_rows(query_rows, &mut rows_by_geo_hash);
            }
            Err(err) if chunk.len() > 1 => {
                eprintln!(
                    "⚠️ primitive_keypoints 批量查询失败，将拆分单条重试: chunk_size={}, error={err:#}",
                    chunk.len()
                );
                for geo_hash in chunk {
                    match query_primitive_keypoint_rows_chunk(std::slice::from_ref(geo_hash)).await
                    {
                        Ok(query_rows) => {
                            merge_primitive_keypoint_query_rows(query_rows, &mut rows_by_geo_hash);
                        }
                        Err(single_err) => {
                            eprintln!(
                                "⚠️ primitive_keypoints 跳过 geo_hash={}，查询失败: {single_err:#}",
                                geo_hash
                            );
                        }
                    }
                }
            }
            Err(err) => {
                if let Some(geo_hash) = chunk.first() {
                    eprintln!(
                        "⚠️ primitive_keypoints 跳过 geo_hash={}，查询失败: {err:#}",
                        geo_hash
                    );
                }
            }
        }
    }

    let mut rows = rows_by_geo_hash
        .into_values()
        .flatten()
        .collect::<Vec<PrimitiveKeyPointRow>>();
    rows.sort_by(|a, b| {
        a.geo_hash
            .cmp(&b.geo_hash)
            .then(a.keypoint_index.cmp(&b.keypoint_index))
    });

    if verbose {
        let with_points = rows
            .iter()
            .map(|row| row.geo_hash.as_str())
            .collect::<HashSet<_>>()
            .len();
        println!(
            "✅ primitive_keypoints.parquet 准备写入: {} / {} 个 geo_hash, {} 个点",
            with_points,
            geo_hashes.len(),
            rows.len()
        );
    }

    Ok(rows)
}

async fn query_primitive_keypoint_rows_chunk(
    chunk: &[String],
) -> Result<Vec<PrimitiveKeyPointQueryRow>> {
    if chunk.is_empty() {
        return Ok(Vec::new());
    }

    let quoted_hashes = chunk
        .iter()
        .map(|hash| format!("'{}'", hash.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        r#"
        SELECT
            record::id(out) AS geo_hash,
            pts[*].d AS pts
        FROM geo_relate
        WHERE visible
          AND record::id(out) IN [{quoted_hashes}]
          AND pts != NONE
        "#
    );

    aios_core::project_primary_db()
        .query_take(&sql, 0)
        .await
        .with_context(|| format!("query_primitive_keypoint_rows chunk_size={}", chunk.len()))
}

fn merge_primitive_keypoint_query_rows(
    query_rows: Vec<PrimitiveKeyPointQueryRow>,
    rows_by_geo_hash: &mut HashMap<String, Vec<PrimitiveKeyPointRow>>,
) {
    for query_row in query_rows {
        if rows_by_geo_hash.contains_key(&query_row.geo_hash) {
            continue;
        }

        let rows = query_row
            .pts
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(idx, point)| {
                let v = point.0;
                PrimitiveKeyPointRow {
                    geo_hash: query_row.geo_hash.clone(),
                    keypoint_index: idx as i32,
                    kind: "key_point".to_string(),
                    local_x: v.x as f64,
                    local_y: v.y as f64,
                    local_z: v.z as f64,
                    has_dir: false,
                    dir_x: 0.0,
                    dir_y: 0.0,
                    dir_z: 0.0,
                    source: "geo_relate.pts".to_string(),
                }
            })
            .collect::<Vec<_>>();

        if !rows.is_empty() {
            rows_by_geo_hash.insert(query_row.geo_hash, rows);
        }
    }
}

/// 本地实现导出专用实例查询。
///
/// 直接内联导出所需的 SurrealQL，避免外部 aios_core 依赖图或 cargo patch
/// 未命中时导致 parquet 导出继续走旧过滤条件。
async fn query_export_insts_local(
    refnos: &[RefnoEnum],
    enable_holes: bool,
    verbose: bool,
) -> Result<Vec<aios_core::ExportInstQuery>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let batch_size = 50;
    let mut results = Vec::new();
    let total_batches = refnos.len().div_ceil(batch_size);
    let query_start = std::time::Instant::now();

    for (batch_idx, chunk) in refnos.chunks(batch_size).enumerate() {
        let batch_start = std::time::Instant::now();
        if enable_holes {
            let bool_keys = chunk
                .iter()
                .map(|r| model_refno_id("inst_relate_bool", *r))
                .collect::<Vec<_>>();
            let bool_keys_str = bool_keys.join(",");

            let bool_sql = format!(
                r#"
                SELECT
                    refno,
                    refno.owner as owner,
                    (if type::record("inst_relate_aabb", id).aabb_id != NONE {{
                        record::id(type::record("inst_relate_aabb", id).aabb_id)
                    }} else {{ None }}) as world_aabb_hash,
                    (if type::record("pe_transform", record::id(refno)).world_trans != NONE {{
                        record::id(type::record("pe_transform", record::id(refno)).world_trans)
                    }} else {{ None }}) as world_trans_hash,
                    [{{ "geo_hash": mesh_id, "trans_hash": "0", "unit_flag": false }}] as insts,
                    true as has_neg
                FROM [{bool_keys}]
                WHERE status = 'Success'
                  AND refno != NONE
                "#,
                bool_keys = bool_keys_str
            );

            let mut bool_results: Vec<aios_core::ExportInstQuery> = aios_core::project_primary_db()
                .query_take(&bool_sql, 0)
                .await
                .with_context(|| format!("query_export_insts_local bool SQL: {bool_sql}"))?;

            let bool_refnos: HashSet<RefnoEnum> = bool_results.iter().map(|r| r.refno).collect();
            results.append(&mut bool_results);

            let non_bool_refnos = chunk
                .iter()
                .filter(|r| !bool_refnos.contains(*r))
                .copied()
                .collect::<Vec<_>>();

            if !non_bool_refnos.is_empty() {
                let mut geo_sql_batch = String::new();
                for r in &non_bool_refnos {
                    let inst_relate_key = model_refno_id("inst_relate", *r);
                    let geo_range = model_refno_range("geo_relate", *r);
                    geo_sql_batch.push_str(&format!(
                        r#"
                        SELECT
                            in as refno,
                            in.owner ?? in as owner,
                            (if type::record("inst_relate_aabb", id).aabb_id != NONE {{
                                record::id(type::record("inst_relate_aabb", id).aabb_id)
                            }} else {{ None }}) as world_aabb_hash,
                            (if type::record("pe_transform", record::id(in)).world_trans != NONE {{
                                record::id(type::record("pe_transform", record::id(in)).world_trans)
                            }} else {{ None }}) as world_trans_hash,
                            (SELECT
                                record::id(trans) as trans_hash,
                                record::id(out) as geo_hash,
                                out.unit_flag ?? false as unit_flag
                             FROM {geo_range}
                             WHERE visible
                               && out != NONE
                               && (out.param != NONE || out.meshed || out.unit_flag || record::id(out) IN ['1','2','3'])
                               && (trans.d ?? NONE) != NONE
                               && geo_type IN ['Pos', 'DesiPos', 'CatePos', 'Compound']) as insts,
                            false as has_neg
                        FROM [{inst_relate_key}]
                        WHERE in != NONE;
                        "#
                    ));
                }

                let mut resp = aios_core::project_primary_db()
                    .query_response(&geo_sql_batch)
                    .await
                    .with_context(|| {
                        format!("query_export_insts_local geo SQL: {geo_sql_batch}")
                    })?;
                for (stmt_idx, _) in non_bool_refnos.iter().enumerate() {
                    let mut geo_results: Vec<aios_core::ExportInstQuery> = resp.take(stmt_idx)?;
                    results.append(&mut geo_results);
                }
            }
        } else {
            let mut sql_batch = String::new();
            for r in chunk {
                let inst_relate_key = model_refno_id("inst_relate", *r);
                let geo_range = model_refno_range("geo_relate", *r);
                sql_batch.push_str(&format!(
                    r#"
                    SELECT
                        in as refno,
                        in.owner ?? in as owner,
                        (if type::record("inst_relate_aabb", id).aabb_id != NONE {{
                            record::id(type::record("inst_relate_aabb", id).aabb_id)
                        }} else {{ None }}) as world_aabb_hash,
                        (if type::record("pe_transform", record::id(in)).world_trans != NONE {{
                            record::id(type::record("pe_transform", record::id(in)).world_trans)
                        }} else {{ None }}) as world_trans_hash,
                        (SELECT
                            record::id(trans) as trans_hash,
                            record::id(out) as geo_hash,
                            out.unit_flag ?? false as unit_flag
                         FROM {geo_range}
                         WHERE visible
                           && out != NONE
                           && (out.param != NONE || out.meshed || out.unit_flag || record::id(out) IN ['1','2','3'])
                           && (trans.d ?? NONE) != NONE
                           && geo_type IN ['Pos', 'DesiPos', 'CatePos', 'Compound']) as insts,
                        false as has_neg
                    FROM [{inst_relate_key}]
                    WHERE in != NONE;
                    "#
                ));
            }

            let mut resp = aios_core::project_primary_db()
                .query_response(&sql_batch)
                .await
                .with_context(|| format!("query_export_insts_local SQL: {sql_batch}"))?;
            for (stmt_idx, _) in chunk.iter().enumerate() {
                let mut chunk_results: Vec<aios_core::ExportInstQuery> = resp.take(stmt_idx)?;
                results.append(&mut chunk_results);
            }
        }

        if verbose && ((batch_idx + 1) % 20 == 0 || batch_idx + 1 == total_batches) {
            println!(
                "   - geo hash batch {}/{}: chunk={}, results_so_far={}, batch_elapsed={:?}, total_elapsed={:?}",
                batch_idx + 1,
                total_batches,
                chunk.len(),
                results.len(),
                batch_start.elapsed(),
                query_start.elapsed()
            );
        }
    }

    if verbose {
        println!(
            "✅ 几何体实例 hash 查询完成: {} / {} refno ({:?})",
            results.len(),
            refnos.len(),
            query_start.elapsed()
        );
    }

    Ok(results)
}

/// 批量查询 tubi_relate
async fn query_tubi_relate(
    owner_refnos: &[RefnoEnum],
    verbose: bool,
) -> Result<HashMap<RefnoEnum, Vec<TubiQueryResult>>> {
    let mut tubings_map: HashMap<RefnoEnum, Vec<TubiQueryResult>> = HashMap::new();

    if owner_refnos.is_empty() {
        return Ok(tubings_map);
    }

    for owners_chunk in owner_refnos.chunks(200) {
        let mut sql_batch = String::new();
        for owner_refno in owners_chunk {
            let tubi_range = model_refno_range("tubi_relate", *owner_refno);
            sql_batch.push_str(&format!(
                r#"
                SELECT
                    {owner_refno} as refno,
                    id[2] as index,
                    in as leave,
                    record::id(aabb) as world_aabb_hash,
                    record::id(world_trans) as world_trans_hash,
                    record::id(geo) as geo_hash,
                    spec_value
                FROM {tubi_range};
                "#,
                owner_refno = owner_refno.to_pe_key()
            ));
        }

        let mut resp = aios_core::project_primary_db()
            .query_response(&sql_batch)
            .await?;
        for (stmt_idx, owner_refno) in owners_chunk.iter().enumerate() {
            let raw_rows: Vec<TubiQueryResult> = resp.take(stmt_idx)?;
            for row in raw_rows {
                if row.geo_hash.is_some() {
                    tubings_map.entry(*owner_refno).or_default().push(row);
                }
            }
        }
    }

    // 排序：按 index 保序
    for tubis in tubings_map.values_mut() {
        tubis.sort_by_key(|t| t.index.unwrap_or(0));
    }

    if verbose {
        let total: usize = tubings_map.values().map(|v| v.len()).sum();
        println!("   ✅ 查询到 {} 条 tubi_relate 记录", total);
    }

    Ok(tubings_map)
}

/// 批量查询 trans 表，返回 TransformRow 列表
async fn query_trans_rows(
    hashes: &HashSet<String>,
    unit_converter: &UnitConverter,
    verbose: bool,
) -> Result<Vec<TransformRow>> {
    use aios_core::project_primary_db;

    let mut result = Vec::new();
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

        let rows: Vec<TransQueryRow> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        for row in rows {
            if let Some(obj) = row.d.as_object() {
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

                // 单位转换（仅平移部分）
                let factor = unit_converter.conversion_factor() as f64;
                let converted_translation = glam::DVec3::new(
                    translation.x * factor,
                    translation.y * factor,
                    translation.z * factor,
                );

                let mat = glam::DMat4::from_scale_rotation_translation(
                    scale,
                    rotation,
                    converted_translation,
                );
                let cols = mat.to_cols_array();

                result.push(TransformRow {
                    trans_hash: row.hash,
                    m00: cols[0],
                    m10: cols[1],
                    m20: cols[2],
                    m30: cols[3],
                    m01: cols[4],
                    m11: cols[5],
                    m21: cols[6],
                    m31: cols[7],
                    m02: cols[8],
                    m12: cols[9],
                    m22: cols[10],
                    m32: cols[11],
                    m03: cols[12],
                    m13: cols[13],
                    m23: cols[14],
                    m33: cols[15],
                });
            }
        }
    }

    Ok(result)
}

/// 批量查询 aabb 表，返回 AabbRow 列表
async fn query_aabb_rows(
    hashes: &HashSet<String>,
    unit_converter: &UnitConverter,
    verbose: bool,
) -> Result<Vec<AabbRow>> {
    use aios_core::project_primary_db;

    let mut result = Vec::new();
    if hashes.is_empty() {
        return Ok(result);
    }

    let hashes_vec: Vec<&String> = hashes.iter().collect();
    for chunk in hashes_vec.chunks(500) {
        let keys: Vec<String> = chunk.iter().map(|h| format!("aabb:⟨{}⟩", h)).collect();
        let sql = format!(
            "SELECT record::id(id) as hash, d FROM [{}]",
            keys.join(", ")
        );

        if verbose {
            println!("   查询 aabb: {} 个", chunk.len());
        }

        let rows: Vec<AabbQueryRow> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        for row in rows {
            if let Some(aabb) = row.d {
                let mins = aabb.0.mins;
                let maxs = aabb.0.maxs;
                let factor = unit_converter.conversion_factor() as f64;
                result.push(AabbRow {
                    aabb_hash: row.hash,
                    min_x: mins.x as f64 * factor,
                    min_y: mins.y as f64 * factor,
                    min_z: mins.z as f64 * factor,
                    max_x: maxs.x as f64 * factor,
                    max_y: maxs.y as f64 * factor,
                    max_z: maxs.z as f64 * factor,
                });
            }
        }
    }

    Ok(result)
}

// =============================================================================
// 主导出函数
// =============================================================================

/// Parquet 导出统计信息
pub struct ParquetExportStats {
    pub instance_count: usize,
    pub ptset_count: usize,
    pub primitive_keypoint_count: usize,
    pub geo_instance_count: usize,
    pub tubing_count: usize,
    pub transform_count: usize,
    pub aabb_count: usize,
    pub spec_info_fallback_count: usize,
    pub total_bytes: u64,
    pub elapsed: std::time::Duration,
}

/// 从 SurrealDB 导出指定 dbnum 的实例数据为多表 Parquet 格式
///
/// # 参数
/// - `dbnum`: 数据库编号
/// - `output_dir`: 输出目录
/// - `db_option`: 数据库选项
/// - `verbose`: 是否输出详细日志
/// - `target_unit`: 目标单位（可选，默认毫米）
/// - `root_refno`: 若提供，则仅导出该 refno 下的 visible 子孙节点
///
/// # 返回
/// 导出统计信息
#[cfg_attr(
    feature = "profile",
    tracing::instrument(skip_all, name = "export_dbnum_instances_parquet")
)]
pub async fn export_dbnum_instances_parquet(
    dbnum: u32,
    output_dir: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    target_unit: Option<LengthUnit>,
    root_refno: Option<RefnoEnum>,
) -> Result<ParquetExportStats> {
    let start_time = std::time::Instant::now();

    let target = target_unit.unwrap_or(LengthUnit::Millimeter);
    let unit_converter = UnitConverter::new(LengthUnit::Millimeter, target);

    if verbose {
        println!(
            "🚀 开始导出 dbnum={} 的实例数据为 Parquet，目标单位: {:?}",
            dbnum, target
        );
    }

    // 确保输出目录存在
    fs::create_dir_all(output_dir)
        .with_context(|| format!("创建输出目录失败: {}", output_dir.display()))?;
    let mesh_base_dir = mesh_base_dir_from_db_option(&db_option);

    // 构建/加载 spec_info（BRAN/HANG/EQUI/WALL/FLOOR 专业信息），用于 spec_value=0 时回填
    // specs/023 M2：层级/元信息读取走 HierView（pe_owner 快照默认 / .tree 回退）
    let tree_manager = HierView::load(vec![dbnum]).await?;
    let tree_dir_buf = TreeIndexManager::with_default_dir(vec![dbnum])
        .tree_dir()
        .to_path_buf();
    let tree_dir = tree_dir_buf.as_path();
    let spec_info_map = match crate::fast_model::export_model::spec_info::load_or_build_spec_info(
        dbnum, tree_dir, output_dir, verbose,
    )
    .await
    {
        Ok(m) => {
            if verbose && !m.is_empty() {
                println!("   📋 spec_info: {} 条 refno->spec_value 映射", m.len());
            }
            m
        }
        Err(e) => {
            eprintln!("   ⚠️ spec_info 加载/构建失败 (将使用 spec_value=0): {}", e);
            HashMap::new()
        }
    };

    // =========================================================================
    // 1-2. 扫描 inst_relate（按 dbnum 对应的 ref0 前缀过滤）
    // =========================================================================
    let (inst_rows, scoped_refno_filter) = if let Some(root) = root_refno {
        // root_refno 模式：先查子树 refno，再分批查 inst_relate
        use crate::fast_model::query_compat::query_deep_visible_inst_refnos;
        if verbose {
            println!("🔍 查询 {} 的可见实例节点...", root);
        }
        let mut sub_refnos = query_deep_visible_inst_refnos(root).await?;
        sub_refnos.push(root);
        sub_refnos.sort();
        sub_refnos.dedup();
        if verbose {
            println!("✅ 子树 refno 数量: {}", sub_refnos.len());
        }
        let scoped_refno_filter = sub_refnos.iter().copied().collect::<HashSet<_>>();
        (
            query_inst_relate_by_refnos(&sub_refnos, verbose).await?,
            Some(scoped_refno_filter),
        )
    } else {
        (query_inst_relate_by_dbnum(dbnum, verbose).await?, None)
    };

    // 按 owner 分组
    struct ChildInfo {
        refno: RefnoEnum,
        noun: String,
        spec_value: i64,
        spec_info_fallback: bool,
        owner_refno: Option<RefnoEnum>,
        owner_type: String,
    }

    let mut grouped_children: HashMap<RefnoEnum, Vec<ChildInfo>> = HashMap::new();
    let mut ungrouped: Vec<ChildInfo> = Vec::new();
    let mut in_refnos: Vec<RefnoEnum> = Vec::new();
    let mut in_refno_set: HashSet<RefnoEnum> = HashSet::new();
    let mut spec_lookup_cache: HashMap<u64, i64> = HashMap::new();

    for row in inst_rows {
        let owner_type = row
            .owner_type
            .as_deref()
            .unwrap_or_default()
            .to_ascii_uppercase();

        let mut spec_value = row.spec_value.unwrap_or(0);
        let mut spec_info_fallback = false;
        if spec_value == 0 {
            spec_value = resolve_spec_value_with_ancestors(
                row.refno,
                row.owner_refno,
                &tree_manager,
                &spec_info_map,
                &mut spec_lookup_cache,
            );
            spec_info_fallback = spec_value == 0;
        }
        let child = ChildInfo {
            refno: row.refno,
            noun: row.noun.unwrap_or_default(),
            spec_value,
            spec_info_fallback,
            owner_refno: row.owner_refno,
            owner_type: owner_type.clone(),
        };

        if in_refno_set.insert(row.refno) {
            in_refnos.push(row.refno);
        }

        if matches!(owner_type.as_str(), "BRAN" | "HANG" | "EQUI") {
            if let Some(owner) = row.owner_refno {
                grouped_children.entry(owner).or_default().push(child);
            } else {
                ungrouped.push(child);
            }
        } else {
            ungrouped.push(child);
        }
    }

    let mut tree_visible_added = 0usize;
    for refno in tree_manager.query_visible_geo_refnos() {
        if let Some(scope) = &scoped_refno_filter
            && !scope.contains(&refno)
        {
            continue;
        }
        if !in_refno_set.insert(refno) {
            continue;
        }

        let Some(meta) = tree_manager.get_node_meta(refno) else {
            continue;
        };
        let owner_refno = if meta.owner.0 == 0 {
            None
        } else {
            Some(RefnoEnum::from(meta.owner))
        };
        let owner_type = owner_refno
            .and_then(|owner| tree_manager.get_noun(owner))
            .unwrap_or_default()
            .to_ascii_uppercase();
        let spec_value = resolve_spec_value_with_ancestors(
            refno,
            owner_refno,
            &tree_manager,
            &spec_info_map,
            &mut spec_lookup_cache,
        );
        let spec_info_fallback = spec_value == 0;

        let child = ChildInfo {
            refno,
            noun: tree_manager.get_noun(refno).unwrap_or_default(),
            spec_value,
            spec_info_fallback,
            owner_refno,
            owner_type: owner_type.clone(),
        };

        in_refnos.push(refno);
        if matches!(owner_type.as_str(), "BRAN" | "HANG" | "EQUI") {
            if let Some(owner) = owner_refno {
                grouped_children.entry(owner).or_default().push(child);
            } else {
                ungrouped.push(child);
            }
        } else {
            ungrouped.push(child);
        }
        tree_visible_added += 1;
    }

    if verbose && tree_visible_added > 0 {
        println!(
            "   ✅ 从 TreeIndex 补齐 visible geo 语义节点: {} 行",
            tree_visible_added
        );
    }

    // =========================================================================
    // 3. 查询几何体实例 hash（geo_relate / inst_relate_bool）
    // =========================================================================
    if verbose {
        println!("🔍 查询 {} 个 refno 的几何体实例 hash...", in_refnos.len());
    }
    let mut export_inst_map: HashMap<RefnoEnum, aios_core::ExportInstQuery> = HashMap::new();
    if !in_refnos.is_empty() {
        match query_export_insts_local(&in_refnos, true, verbose).await {
            Ok(export_insts) => {
                for inst in export_insts {
                    export_inst_map.insert(inst.refno, inst);
                }
                if verbose {
                    println!(
                        "✅ 查询到 {} 个 refno 有几何体实例 (inst_relate 共 {} 个)",
                        export_inst_map.len(),
                        in_refnos.len()
                    );
                    let in_set: HashSet<_> = in_refnos.iter().collect();
                    let exported_set: HashSet<_> = export_inst_map.keys().collect();
                    let missing_geo: Vec<_> = in_set.difference(&exported_set).collect();
                    if !missing_geo.is_empty() && missing_geo.len() <= 20 {
                        println!(
                            "   ⚠️ 以下 refno 在 inst_relate 但无几何体(geo_relate/inst_relate_bool): {:?}",
                            missing_geo
                                .iter()
                                .map(|r| r.to_string())
                                .collect::<Vec<_>>()
                        );
                    } else if !missing_geo.is_empty() {
                        println!(
                            "   ⚠️ {} 个 refno 在 inst_relate 但无几何体，样例: {:?}",
                            missing_geo.len(),
                            missing_geo
                                .iter()
                                .take(5)
                                .map(|r| r.to_string())
                                .collect::<Vec<_>>()
                        );
                    }
                }
            }
            Err(e) => {
                if verbose {
                    println!("⚠️ 几何体实例查询失败: {:?}", e);
                }
            }
        }
    }

    // =========================================================================
    // 3.5 查询 inst_info cata_hash / ptset（供 instances 与 ptsets.parquet 复用）
    // =========================================================================
    if verbose {
        println!(
            "🔍 查询 {} 个导出实例 refno 的 cata_hash / ptset...",
            in_refnos.len()
        );
    }
    let ptset_export_data = query_ptset_export_data(&in_refnos, verbose).await?;

    // =========================================================================
    // 4. 查询 tubi_relate
    // =========================================================================
    let mut tubi_owner_refnos: Vec<RefnoEnum> = grouped_children
        .iter()
        .filter(|(_, children)| {
            children
                .first()
                .map_or(false, |c| matches!(c.owner_type.as_str(), "BRAN" | "HANG"))
        })
        .map(|(k, _)| *k)
        .collect();

    if let Some(root) = root_refno {
        let root_noun = tree_manager
            .get_noun(root)
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();
        if matches!(root_noun.as_str(), "BRAN" | "HANG") && !tubi_owner_refnos.contains(&root) {
            tubi_owner_refnos.push(root);
        }
    }

    if verbose {
        println!(
            "🔍 查询 {} 个 BRAN/HANG owner 的 tubi_relate...",
            tubi_owner_refnos.len()
        );
    }
    let tubings_map = query_tubi_relate(&tubi_owner_refnos, verbose).await?;

    // =========================================================================
    // 5. 构建 Parquet 行数据
    // =========================================================================
    let mut instance_rows: Vec<InstanceRow> = Vec::new();
    let mut geo_instance_rows: Vec<GeoInstanceRow> = Vec::new();
    let mut tubing_rows: Vec<TubingRow> = Vec::new();
    let mut trans_hashes: HashSet<String> = HashSet::new();
    let mut aabb_hashes: HashSet<String> = HashSet::new();
    let mut owner_refnos_by_hash: HashMap<String, HashSet<String>> = HashMap::new();
    let mut row_count_by_hash: HashMap<String, usize> = HashMap::new();

    let mut emitted_tubing_owners: HashSet<RefnoEnum> = HashSet::new();
    let mut emitted_instance_refnos: HashSet<RefnoEnum> = HashSet::new();

    // 处理 grouped children
    for (owner_refno, children) in &grouped_children {
        let owner_type = children
            .first()
            .map(|c| c.owner_type.as_str())
            .unwrap_or("");

        for child in children {
            let export_inst = export_inst_map.get(&child.refno);
            let child_aabb_hash = export_inst
                .and_then(|inst| inst.world_aabb_hash.clone())
                .unwrap_or_default();
            let trans_hash = export_inst
                .and_then(|inst| inst.world_trans_hash.clone())
                .unwrap_or_default();
            let has_neg = export_inst.map(|inst| inst.has_neg).unwrap_or(false);

            // 收集 hash
            if !child_aabb_hash.is_empty() {
                aabb_hashes.insert(child_aabb_hash.clone());
            }
            if !trans_hash.is_empty() {
                trans_hashes.insert(trans_hash.clone());
            }
            if let Some(export_inst) = export_inst {
                for inst in &export_inst.insts {
                    if let Some(ref th) = inst.trans_hash {
                        if !th.is_empty() {
                            trans_hashes.insert(th.clone());
                        }
                    }
                }
            }

            if emitted_instance_refnos.insert(child.refno) {
                instance_rows.push(InstanceRow {
                    refno_str: child.refno.to_string(),
                    refno_u64: refno_to_u64(&child.refno),
                    noun: child.noun.clone(),
                    owner_refno_str: Some(owner_refno.to_string()),
                    owner_refno_u64: Some(refno_to_u64(owner_refno)),
                    owner_noun: owner_type.to_string(),
                    cata_hash: ptset_export_data.refno_cata_hash.get(&child.refno).cloned(),
                    trans_hash: trans_hash.clone(),
                    aabb_hash: child_aabb_hash,
                    spec_value: child.spec_value,
                    spec_info_fallback: child.spec_info_fallback,
                    has_neg,
                    dbnum,
                });
            }

            // geo_instances
            if let Some(export_inst) = export_inst {
                for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                    geo_instance_rows.push(GeoInstanceRow {
                        refno_str: child.refno.to_string(),
                        refno_u64: refno_to_u64(&child.refno),
                        geo_index: geo_idx as u32,
                        geo_hash: inst.geo_hash.clone(),
                        geo_trans_hash: inst.trans_hash.clone().unwrap_or_default(),
                    });
                    record_geo_hash_usage(
                        &inst.geo_hash,
                        &child.refno.to_string(),
                        &mut owner_refnos_by_hash,
                        &mut row_count_by_hash,
                    );
                }
            }
        }

        // tubings
        if let Some(tubis) = tubings_map.get(owner_refno) {
            append_tubing_rows_for_owner(
                owner_refno,
                tubis,
                &spec_info_map,
                &tree_manager,
                &mut spec_lookup_cache,
                dbnum,
                &mut tubing_rows,
                &mut trans_hashes,
                &mut aabb_hashes,
                &mut owner_refnos_by_hash,
                &mut row_count_by_hash,
            );
            emitted_tubing_owners.insert(*owner_refno);
        }
    }

    for owner_refno in &tubi_owner_refnos {
        if emitted_tubing_owners.contains(owner_refno) {
            continue;
        }
        if let Some(tubis) = tubings_map.get(owner_refno) {
            append_tubing_rows_for_owner(
                owner_refno,
                tubis,
                &spec_info_map,
                &tree_manager,
                &mut spec_lookup_cache,
                dbnum,
                &mut tubing_rows,
                &mut trans_hashes,
                &mut aabb_hashes,
                &mut owner_refnos_by_hash,
                &mut row_count_by_hash,
            );
        }
    }

    // 处理 ungrouped instances
    for child in &ungrouped {
        let export_inst = export_inst_map.get(&child.refno);
        let child_aabb_hash = export_inst
            .and_then(|inst| inst.world_aabb_hash.clone())
            .unwrap_or_default();
        let trans_hash = export_inst
            .and_then(|inst| inst.world_trans_hash.clone())
            .unwrap_or_default();
        let has_neg = export_inst.map(|inst| inst.has_neg).unwrap_or(false);

        if !child_aabb_hash.is_empty() {
            aabb_hashes.insert(child_aabb_hash.clone());
        }
        if !trans_hash.is_empty() {
            trans_hashes.insert(trans_hash.clone());
        }
        if let Some(export_inst) = export_inst {
            for inst in &export_inst.insts {
                if let Some(ref th) = inst.trans_hash {
                    if !th.is_empty() {
                        trans_hashes.insert(th.clone());
                    }
                }
            }
        }

        if emitted_instance_refnos.insert(child.refno) {
            instance_rows.push(InstanceRow {
                refno_str: child.refno.to_string(),
                refno_u64: refno_to_u64(&child.refno),
                noun: child.noun.clone(),
                owner_refno_str: child.owner_refno.map(|r| r.to_string()),
                owner_refno_u64: child.owner_refno.map(|r| refno_to_u64(&r)),
                owner_noun: child.owner_type.clone(),
                cata_hash: ptset_export_data.refno_cata_hash.get(&child.refno).cloned(),
                trans_hash: trans_hash.clone(),
                aabb_hash: child_aabb_hash,
                spec_value: child.spec_value,
                spec_info_fallback: child.spec_info_fallback,
                has_neg,
                dbnum,
            });
        }

        if let Some(export_inst) = export_inst {
            for (geo_idx, inst) in export_inst.insts.iter().enumerate() {
                geo_instance_rows.push(GeoInstanceRow {
                    refno_str: child.refno.to_string(),
                    refno_u64: refno_to_u64(&child.refno),
                    geo_index: geo_idx as u32,
                    geo_hash: inst.geo_hash.clone(),
                    geo_trans_hash: inst.trans_hash.clone().unwrap_or_default(),
                });
                record_geo_hash_usage(
                    &inst.geo_hash,
                    &child.refno.to_string(),
                    &mut owner_refnos_by_hash,
                    &mut row_count_by_hash,
                );
            }
        }
    }

    let missing_mesh_report = write_missing_mesh_report(
        output_dir,
        dbnum,
        &mesh_base_dir,
        MESH_CHECK_LOD_TAG,
        &owner_refnos_by_hash,
        &row_count_by_hash,
        verbose,
    )?;

    let drop_missing_mesh_rows = std::env::var("AIOS_PARQUET_DROP_MISSING_MESH_ROWS")
        .ok()
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    let mut mesh_validation_summary = MeshValidationExportSummary {
        policy: "retain_missing_mesh_rows",
        raw_checked_geo_hashes: missing_mesh_report.checked_geo_hashes,
        raw_missing_geo_hashes: missing_mesh_report.missing_geo_hashes,
        raw_missing_owner_refnos: missing_mesh_report.missing_owner_refnos,
        render_missing_geo_hashes: missing_mesh_report.missing_geo_hashes,
        render_missing_owner_refnos: missing_mesh_report.missing_owner_refnos,
        quarantined_geo_hashes: 0,
        quarantined_owner_refnos: 0,
        dropped_geo_instance_rows: 0,
        dropped_tubing_rows: 0,
        dropped_instance_rows: 0,
    };
    if drop_missing_mesh_rows && !missing_mesh_report.missing_geo_hash_values.is_empty() {
        let before_geo_rows = geo_instance_rows.len();
        let before_tubing_rows = tubing_rows.len();
        let before_instance_rows = instance_rows.len();

        geo_instance_rows.retain(|row| {
            !missing_mesh_report
                .missing_geo_hash_values
                .contains(row.geo_hash.trim())
        });
        tubing_rows.retain(|row| {
            !missing_mesh_report
                .missing_geo_hash_values
                .contains(row.geo_hash.trim())
        });

        let renderable_refnos = geo_instance_rows
            .iter()
            .map(|row| row.refno_str.clone())
            .chain(tubing_rows.iter().map(|row| row.owner_refno_str.clone()))
            .collect::<HashSet<_>>();
        instance_rows.retain(|row| renderable_refnos.contains(&row.refno_str));

        mesh_validation_summary = MeshValidationExportSummary {
            policy: "quarantine_missing_mesh_rows",
            raw_checked_geo_hashes: missing_mesh_report.checked_geo_hashes,
            raw_missing_geo_hashes: missing_mesh_report.missing_geo_hashes,
            raw_missing_owner_refnos: missing_mesh_report.missing_owner_refnos,
            render_missing_geo_hashes: 0,
            render_missing_owner_refnos: 0,
            quarantined_geo_hashes: missing_mesh_report.missing_geo_hashes,
            quarantined_owner_refnos: missing_mesh_report.missing_owner_refnos,
            dropped_geo_instance_rows: before_geo_rows.saturating_sub(geo_instance_rows.len()),
            dropped_tubing_rows: before_tubing_rows.saturating_sub(tubing_rows.len()),
            dropped_instance_rows: before_instance_rows.saturating_sub(instance_rows.len()),
        };

        if verbose {
            println!(
                "   ⚠️ 已从 Parquet 渲染表剔除缺失 GLB: geo_rows {} -> {}, tubings {} -> {}, instances {} -> {}",
                before_geo_rows,
                geo_instance_rows.len(),
                before_tubing_rows,
                tubing_rows.len(),
                before_instance_rows,
                instance_rows.len()
            );
        }
    } else if !missing_mesh_report.missing_geo_hash_values.is_empty() && verbose {
        println!(
            "   ⚠️ 检测到缺失 GLB，但保留 Parquet 语义行: geo_hashes={}, owner_refnos={} (设置 AIOS_PARQUET_DROP_MISSING_MESH_ROWS=1 可恢复过滤)",
            missing_mesh_report.missing_geo_hashes, missing_mesh_report.missing_owner_refnos
        );
    }

    append_owner_chain_rows(
        &mut instance_rows,
        &tree_manager,
        &spec_info_map,
        &mut spec_lookup_cache,
        dbnum,
        root_refno,
        verbose,
    );

    let used_cata_hashes = instance_rows
        .iter()
        .filter_map(|row| row.cata_hash.as_ref())
        .cloned()
        .collect::<HashSet<_>>();
    let mut ptset_rows = used_cata_hashes
        .iter()
        .filter_map(|cata_hash| ptset_export_data.rows_by_cata_hash.get(cata_hash))
        .flat_map(|rows| rows.iter().cloned())
        .collect::<Vec<_>>();
    ptset_rows.sort_by(|a, b| {
        a.cata_hash
            .cmp(&b.cata_hash)
            .then(a.point_number.cmp(&b.point_number))
    });

    if verbose {
        println!(
            "✅ ptsets.parquet 准备写入: {} 个 cata_hash, {} 个点",
            used_cata_hashes.len(),
            ptset_rows.len()
        );
    }

    let instance_spec_info_fallback_rows = instance_rows
        .iter()
        .filter(|row| row.spec_info_fallback)
        .count();
    let tubing_spec_info_fallback_rows = tubing_rows
        .iter()
        .filter(|row| row.spec_info_fallback)
        .count();
    let spec_info_fallback_count =
        instance_spec_info_fallback_rows + tubing_spec_info_fallback_rows;

    let used_geo_hashes = geo_instance_rows
        .iter()
        .map(|row| row.geo_hash.clone())
        .chain(tubing_rows.iter().map(|row| row.geo_hash.clone()))
        .filter(|hash| !hash.trim().is_empty())
        .collect::<HashSet<_>>();
    let primitive_keypoint_rows = query_primitive_keypoint_rows(&used_geo_hashes, verbose).await?;

    // =========================================================================
    // 6. 查询 trans/aabb 实际数据
    // =========================================================================
    if verbose {
        println!(
            "🔍 查询 {} 个 trans, {} 个 aabb...",
            trans_hashes.len(),
            aabb_hashes.len()
        );
    }
    let (transform_rows_result, aabb_rows_result) = tokio::join!(
        query_trans_rows(&trans_hashes, &unit_converter, verbose),
        query_aabb_rows(&aabb_hashes, &unit_converter, verbose),
    );
    let transform_rows = transform_rows_result?;
    let aabb_row_data = aabb_rows_result?;

    if verbose {
        println!(
            "✅ trans 命中: {}, aabb 命中: {}",
            transform_rows.len(),
            aabb_row_data.len()
        );
    }

    // =========================================================================
    // 7. 写入 Parquet 文件
    // =========================================================================
    if verbose {
        println!("\n📝 写入 Parquet 文件...");
    }

    let mut total_bytes: u64 = 0;

    // instances.parquet
    {
        let batch = build_instances_batch(&instance_rows)?;
        let path = output_dir.join("instances.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ instances.parquet: {} 行, {} 字节",
                instance_rows.len(),
                size
            );
        }
    }

    // ptsets.parquet
    {
        let batch = build_ptsets_batch(&ptset_rows)?;
        let path = output_dir.join("ptsets.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ ptsets.parquet: {} 行, {} 字节",
                ptset_rows.len(),
                size
            );
        }
    }

    // primitive_keypoints.parquet
    {
        let batch = build_primitive_keypoints_batch(&primitive_keypoint_rows)?;
        let path = output_dir.join("primitive_keypoints.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ primitive_keypoints.parquet: {} 行, {} 字节",
                primitive_keypoint_rows.len(),
                size
            );
        }
    }

    // geo_instances.parquet
    {
        let batch = build_geo_instances_batch(&geo_instance_rows)?;
        let path = output_dir.join("geo_instances.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ geo_instances.parquet: {} 行, {} 字节",
                geo_instance_rows.len(),
                size
            );
        }
    }

    // tubings.parquet
    {
        let batch = build_tubings_batch(&tubing_rows)?;
        let path = output_dir.join("tubings.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ tubings.parquet: {} 行, {} 字节",
                tubing_rows.len(),
                size
            );
        }
    }

    // transforms.parquet
    {
        let batch = build_transforms_batch(&transform_rows)?;
        let path = output_dir.join("transforms.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ transforms.parquet: {} 行, {} 字节",
                transform_rows.len(),
                size
            );
        }
    }

    // aabb.parquet
    {
        let batch = build_aabb_batch(&aabb_row_data)?;
        let path = output_dir.join("aabb.parquet");
        let size = write_parquet(&path, &batch)?;
        total_bytes += size;
        if verbose {
            println!(
                "   ✅ aabb.parquet: {} 行, {} 字节",
                aabb_row_data.len(),
                size
            );
        }
    }

    // =========================================================================
    // 8. 写入 manifest.json
    // =========================================================================
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let manifest = json!({
        "version": 1,
        "format": "parquet",
        "generated_at": generated_at,
        "dbnum": dbnum,
        "root_refno": root_refno.map(|r| r.to_string()),
        "tables": {
            "instances": {
                "file": "instances.parquet",
                "rows": instance_rows.len(),
            },
            "ptsets": {
                "file": "ptsets.parquet",
                "rows": ptset_rows.len(),
                "key": ["cata_hash", "point_number"],
            },
            "primitive_keypoints": {
                "file": "primitive_keypoints.parquet",
                "rows": primitive_keypoint_rows.len(),
                "key": ["geo_hash", "keypoint_index"],
            },
            "geo_instances": {
                "file": "geo_instances.parquet",
                "rows": geo_instance_rows.len(),
            },
            "tubings": {
                "file": "tubings.parquet",
                "rows": tubing_rows.len(),
            },
            "transforms": {
                "file": "transforms.parquet",
                "rows": transform_rows.len(),
            },
            "aabb": {
                "file": "aabb.parquet",
                "rows": aabb_row_data.len(),
            },
        },
        "mesh_validation": {
            "lod_tag": MESH_CHECK_LOD_TAG,
            "report_file": missing_mesh_report.report_file,
            "policy": mesh_validation_summary.policy,
            "checked_geo_hashes": mesh_validation_summary.raw_checked_geo_hashes,
            "missing_geo_hashes": mesh_validation_summary.raw_missing_geo_hashes,
            "missing_owner_refnos": mesh_validation_summary.raw_missing_owner_refnos,
            "raw_checked_geo_hashes": mesh_validation_summary.raw_checked_geo_hashes,
            "raw_missing_geo_hashes": mesh_validation_summary.raw_missing_geo_hashes,
            "raw_missing_owner_refnos": mesh_validation_summary.raw_missing_owner_refnos,
            "render_missing_geo_hashes": mesh_validation_summary.render_missing_geo_hashes,
            "render_missing_owner_refnos": mesh_validation_summary.render_missing_owner_refnos,
            "quarantined_geo_hashes": mesh_validation_summary.quarantined_geo_hashes,
            "quarantined_owner_refnos": mesh_validation_summary.quarantined_owner_refnos,
            "dropped_geo_instance_rows": mesh_validation_summary.dropped_geo_instance_rows,
            "dropped_tubing_rows": mesh_validation_summary.dropped_tubing_rows,
            "dropped_instance_rows": mesh_validation_summary.dropped_instance_rows,
        },
        "spec_info_validation": {
            "fallback_count": spec_info_fallback_count,
            "instance_fallback_rows": instance_spec_info_fallback_rows,
            "tubing_fallback_rows": tubing_spec_info_fallback_rows,
            "definition": "raw/default zero spec_value unresolved by spec_info self, owner, or ancestor lookup",
        },
        "spec_info_fallback_count": spec_info_fallback_count,
        "ptset_unit": {
            "source": LengthUnit::Millimeter.name(),
            "target": target.name(),
            "conversion_factor": unit_converter.conversion_factor(),
            "coordinate_space": "local",
        },
        "primitive_keypoint_unit": {
            "source": LengthUnit::Millimeter.name(),
            "target": target.name(),
            "conversion_factor": unit_converter.conversion_factor(),
            "coordinate_space": "geo_local",
        },
        "ptset_export": {
            "cata_hashes": used_cata_hashes.len(),
            "used_cata_hashes": used_cata_hashes.len(),
            "available_cata_hashes": ptset_export_data.rows_by_cata_hash.len(),
            "refno_cata_hashes": ptset_export_data.refno_cata_hash.len(),
            "requested_refnos": ptset_export_data.requested_refnos,
            "relation_rows": ptset_export_data.relation_rows,
            "inst_info_rows": ptset_export_data.inst_info_rows,
            "written_ptset_points": ptset_rows.len(),
            "invalid_cata_hash_rows": ptset_export_data.invalid_cata_hash_rows,
            "missing_cata_hash_refnos": ptset_export_data.missing_cata_hash_refnos,
            "empty_ptset_hashes": ptset_export_data.empty_ptset_hashes,
        },
        "total_bytes": total_bytes,
    });

    let manifest_path = output_dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)?;
    if verbose {
        println!("   ✅ manifest.json 已写入");
    }

    let elapsed = start_time.elapsed();

    Ok(ParquetExportStats {
        instance_count: instance_rows.len(),
        ptset_count: ptset_rows.len(),
        primitive_keypoint_count: primitive_keypoint_rows.len(),
        geo_instance_count: geo_instance_rows.len(),
        tubing_count: tubing_rows.len(),
        transform_count: transform_rows.len(),
        aabb_count: aabb_row_data.len(),
        spec_info_fallback_count,
        total_bytes,
        elapsed,
    })
}

// =============================================================================
// Cache → Parquet 导出
// =============================================================================
pub async fn export_dbnum_instances_parquet_latest(
    dbnum: u32,
    latest_root: &Path,
    db_option: Arc<DbOption>,
    verbose: bool,
    target_unit: Option<LengthUnit>,
) -> Result<(ParquetExportStats, PathBuf)> {
    let dbnum_root = latest_root.join(dbnum.to_string());
    fs::create_dir_all(&dbnum_root)?;
    let generation = format!(
        "generation-{}-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let staging_dir = dbnum_root.join(format!(".{generation}.staging"));
    let artifact_dir = dbnum_root.join(&generation);

    let stats = match export_dbnum_instances_parquet(
        dbnum,
        &staging_dir,
        db_option,
        verbose,
        target_unit,
        None,
    )
    .await
    {
        Ok(stats) => stats,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(error);
        }
    };
    if let Err(error) = fs::rename(&staging_dir, &artifact_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error).with_context(|| {
            format!(
                "发布 dbnum generation 目录失败: {} -> {}",
                staging_dir.display(),
                artifact_dir.display()
            )
        });
    }
    publish_dbnum_latest_manifest(&artifact_dir, latest_root, dbnum)?;
    Ok((stats, artifact_dir))
}

#[cfg(test)]
mod latest_manifest_tests {
    use super::*;

    fn write_local_manifest(artifact_dir: &Path, root_refno: serde_json::Value) {
        fs::create_dir_all(artifact_dir).expect("artifact dir");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "refno",
            DataType::Utf8,
            false,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(StringArray::from(vec!["24381_145018"]))],
        )
        .expect("record batch");
        let file = fs::File::create(artifact_dir.join("instances.parquet")).expect("table");
        let mut writer = ArrowWriter::try_new(file, schema, None).expect("parquet writer");
        writer.write(&batch).expect("parquet batch");
        writer.close().expect("parquet close");
        fs::write(
            artifact_dir.join("manifest.json"),
            serde_json::to_vec(&json!({
                "version": 1,
                "format": "parquet",
                "generated_at": "2026-07-22T00:00:00Z",
                "dbnum": 7997,
                "root_refno": root_refno,
                "tables": {
                    "instances": {"file": "instances.parquet", "rows": 1}
                }
            }))
            .expect("manifest json"),
        )
        .expect("manifest");
    }

    #[test]
    fn latest_pointer_rejects_root_scoped_artifact_without_replacing_previous() {
        let temp = tempfile::tempdir().expect("tempdir");
        let latest_root = temp.path().join("parquet");
        let artifact_dir = latest_root.join("7997/generation-a");
        write_local_manifest(&artifact_dir, json!("24381_145018"));
        let pointer = latest_root.join("manifest_7997.json");
        fs::write(&pointer, br#"{"generation":"previous"}"#).expect("old pointer");

        assert!(publish_dbnum_latest_manifest(&artifact_dir, &latest_root, 7997).is_err());
        assert_eq!(
            fs::read_to_string(pointer).expect("preserved pointer"),
            r#"{"generation":"previous"}"#
        );
    }

    #[test]
    fn latest_pointer_rewrites_table_paths_to_one_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let latest_root = temp.path().join("parquet");
        let artifact_dir = latest_root.join("7997/generation-a");
        write_local_manifest(&artifact_dir, serde_json::Value::Null);
        let old_pointer = latest_root.join("manifest_7997.json");
        fs::write(&old_pointer, br#"{"generation":"previous"}"#).expect("old pointer");

        let pointer = publish_dbnum_latest_manifest(&artifact_dir, &latest_root, 7997)
            .expect("publish latest");
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(pointer).expect("pointer bytes"))
                .expect("pointer json");
        assert_eq!(
            manifest
                .pointer("/tables/instances/file")
                .and_then(|value| value.as_str()),
            Some("7997/generation-a/instances.parquet")
        );
        assert_ne!(
            fs::read_to_string(old_pointer).expect("new pointer"),
            r#"{"generation":"previous"}"#
        );
    }

    #[test]
    fn latest_pointer_preserves_previous_when_parquet_rows_do_not_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let latest_root = temp.path().join("parquet");
        let artifact_dir = latest_root.join("7997/generation-a");
        write_local_manifest(&artifact_dir, serde_json::Value::Null);
        let local_manifest = artifact_dir.join("manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&local_manifest).expect("manifest bytes"))
                .expect("manifest json");
        manifest["tables"]["instances"]["rows"] = json!(2);
        fs::write(
            local_manifest,
            serde_json::to_vec(&manifest).expect("manifest json"),
        )
        .expect("manifest");
        let pointer = latest_root.join("manifest_7997.json");
        fs::write(&pointer, br#"{"generation":"previous"}"#).expect("old pointer");

        assert!(publish_dbnum_latest_manifest(&artifact_dir, &latest_root, 7997).is_err());
        assert_eq!(
            fs::read_to_string(pointer).expect("preserved pointer"),
            r#"{"generation":"previous"}"#
        );
    }
}
